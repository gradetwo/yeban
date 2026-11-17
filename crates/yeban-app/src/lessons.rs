//! 教学系统：课程数据格式 + 完成判定。
//!
//! # 数据格式
//!
//! 一节课 = [`Lesson`]（id / 标题 / 简介 / 步列表）；一步 = [`LessonStep`]
//! （id / 标题 / 正文 / 判定 [`LessonCheck`]）。格式是纯 serde 数据：
//! 内置课程以 JSON 字面量写在本文件里、经同一条反序列化路径加载，
//! 后续从文件或服务端下发课程不需要改判定代码。
//!
//! # 完成判定（本线的硬约束）
//!
//! 判定**只读真实工程状态**（`YebanProjectV1`），而且是**相对开课基准**的：
//! 开课时把当时已有的轨道 id 集合记进 `help_state::ActiveLesson`，
//! 判定只认基准之外的新轨道与新轨道上的音符。这样演示工程自带的音符
//! 不会让任何一步假通过；点「下一步」本身永远不推进，只有判定通过才推进。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use yeban_model::ids::EntityId;
use yeban_model::project::{ClipContent, TrackKind, YebanProjectV1};

/// 一小节的 tick 数（960 PPQ × 4 拍）。
pub const TICKS_PER_BAR: u64 = 3840;

/// 一节课。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lesson {
    /// 课程 id（进度持久化按它键存）。
    pub id: String,
    /// 课程标题。
    pub title: String,
    /// 一句话简介（课程列表里显示）。
    pub summary: String,
    /// 步骤列表，顺序即教学顺序。
    pub steps: Vec<LessonStep>,
}

/// 一步教学。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonStep {
    /// 步 id（课内唯一）。
    pub id: String,
    /// 步标题。
    pub title: String,
    /// 步正文：让用户做什么、去哪里做。
    pub body: String,
    /// 完成判定。
    pub check: LessonCheck,
}

/// 完成判定的种类。新增判定种类在这里加变体 + 在 [`evaluate`] 里加一条臂，
/// UI 与持久化都不需要改。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LessonCheck {
    /// 基准之外新建了至少一条指定类型的轨道。
    NewTrack {
        /// 轨道类型：`midi` / `audio`（与 `TrackKind` 的可建类型对应）。
        track_kind: RequiredTrackKind,
    },
    /// 基准之外的新 MIDI 轨上写够了音符（数量 / 不同音高数 / 时间跨度，
    /// 可选音高范围——鼓课用它把判定限制在 GM 鼓音高区）。
    NotesOnNewTrack {
        /// 最少音符数（落在音高范围内的才计）。
        min_notes: u32,
        /// 最少不同音高数。
        min_distinct_pitches: u32,
        /// 最早音符起点到最晚音符终点的最少跨度（tick）。
        min_span_ticks: u64,
        /// 音高下限（含）；`None` = 不限。
        #[serde(default)]
        pitch_low: Option<u8>,
        /// 音高上限（含）；`None` = 不限。
        #[serde(default)]
        pitch_high: Option<u8>,
    },
}

/// [`LessonCheck::NewTrack`] 认的轨道类型（用户能新建的两类）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredTrackKind {
    /// MIDI 轨。
    Midi,
    /// 音频轨。
    Audio,
}

impl RequiredTrackKind {
    fn matches(self, kind: TrackKind) -> bool {
        matches!(
            (self, kind),
            (Self::Midi, TrackKind::Midi) | (Self::Audio, TrackKind::Audio)
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::Midi => "MIDI",
            Self::Audio => "音频",
        }
    }
}

/// 一次判定的结论：过没过 + 给用户看的实测进度（数字全部来自工程实测，
/// 不是写死的文案）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckVerdict {
    /// 是否通过。
    pub done: bool,
    /// 实测进度文案，例如「新轨音符 3/8 · 不同音高 2/4 · 跨度 0.5/2 小节」。
    pub progress: String,
}

/// 对当前工程跑一次判定。`baseline_track_ids` 是开课时的轨道 id 集合。
#[must_use]
pub fn evaluate(
    check: &LessonCheck,
    baseline_track_ids: &BTreeSet<EntityId>,
    project: &YebanProjectV1,
) -> CheckVerdict {
    match check {
        LessonCheck::NewTrack { track_kind } => {
            let new = new_tracks(project, baseline_track_ids)
                .filter(|track| track_kind.matches(track.kind))
                .count();
            CheckVerdict {
                done: new >= 1,
                progress: format!("已新建 {} 轨 {new} 条", track_kind.label()),
            }
        }
        LessonCheck::NotesOnNewTrack {
            min_notes,
            min_distinct_pitches,
            min_span_ticks,
            pitch_low,
            pitch_high,
        } => {
            let stats = new_track_note_stats(project, baseline_track_ids, *pitch_low, *pitch_high);
            let done = stats.count >= *min_notes
                && stats.distinct_pitches >= *min_distinct_pitches
                && stats.span_ticks >= *min_span_ticks;
            CheckVerdict {
                done,
                progress: format!(
                    "新轨音符 {}/{} · 不同音高 {}/{} · 跨度 {:.1}/{:.1} 小节",
                    stats.count,
                    min_notes,
                    stats.distinct_pitches,
                    min_distinct_pitches,
                    stats.span_ticks as f64 / TICKS_PER_BAR as f64,
                    *min_span_ticks as f64 / TICKS_PER_BAR as f64,
                ),
            }
        }
    }
}

fn new_tracks<'p>(
    project: &'p YebanProjectV1,
    baseline_track_ids: &BTreeSet<EntityId>,
) -> impl Iterator<Item = &'p yeban_model::project::TrackV3> {
    project
        .tracks
        .values()
        .filter(move |track| !baseline_track_ids.contains(&track.id))
}

#[derive(Default)]
struct NoteStats {
    count: u32,
    distinct_pitches: u32,
    span_ticks: u64,
}

/// 汇总基准之外新 MIDI 轨上的音符：数量、不同音高数、时间跨度。
/// 音符的绝对位置 = 片段摆放起点 + 音符在片段内的起点。
fn new_track_note_stats(
    project: &YebanProjectV1,
    baseline_track_ids: &BTreeSet<EntityId>,
    pitch_low: Option<u8>,
    pitch_high: Option<u8>,
) -> NoteStats {
    let mut pitches: BTreeSet<u8> = BTreeSet::new();
    let mut count: u32 = 0;
    let mut earliest: Option<u64> = None;
    let mut latest: u64 = 0;
    for track in new_tracks(project, baseline_track_ids) {
        if track.kind != TrackKind::Midi {
            continue;
        }
        for placement in track.clips.values() {
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let ClipContent::Midi { notes } = &entry.content else {
                continue;
            };
            for note in notes.values() {
                if pitch_low.is_some_and(|low| note.pitch < low)
                    || pitch_high.is_some_and(|high| note.pitch > high)
                {
                    continue;
                }
                count += 1;
                pitches.insert(note.pitch);
                let start = placement.start_tick + note.start_tick;
                let end = start + note.duration_ticks;
                earliest = Some(earliest.map_or(start, |old: u64| old.min(start)));
                latest = latest.max(end);
            }
        }
    }
    NoteStats {
        count,
        distinct_pitches: u32::try_from(pitches.len()).unwrap_or(u32::MAX),
        span_ticks: earliest.map_or(0, |start| latest.saturating_sub(start)),
    }
}

/// 内置课程表（JSON 字面量 → 同一条反序列化路径，见模块头）。
const CATALOG_JSON: &str = r#"[
  {
    "id": "first-melody",
    "title": "写第一段旋律",
    "summary": "新建一条 MIDI 轨，在钢琴卷帘里写出一段两小节的旋律。",
    "steps": [
      {
        "id": "new-track",
        "title": "新建一条 MIDI 轨",
        "body": "从「轨道」菜单选「+ 新建 MIDI 合成器轨」（Ctrl+N）。新轨会出现在编曲区里。本步只认现在新建的轨道，工程里原有的轨道不算。",
        "check": { "kind": "new_track", "track_kind": "midi" }
      },
      {
        "id": "first-notes",
        "title": "写出前四个音",
        "body": "在编曲区选中新轨的片段，打开钢琴卷帘（P），用绘制工具（数字键 2）点下至少 4 个音：用到至少 3 个不同音高，前后跨度至少 1 小节。",
        "check": { "kind": "notes_on_new_track", "min_notes": 4, "min_distinct_pitches": 3, "min_span_ticks": 3840 }
      },
      {
        "id": "full-phrase",
        "title": "把旋律写成完整一句",
        "body": "在同一条新轨上继续写：至少 8 个音、4 个不同音高、跨度至少 2 小节。写完按 Space 播放一遍，听听它是不是一句完整的旋律。",
        "check": { "kind": "notes_on_new_track", "min_notes": 8, "min_distinct_pitches": 4, "min_span_ticks": 7680 }
      }
    ]
  },
  {
    "id": "first-drum-track",
    "title": "编第一条鼓轨",
    "summary": "用一条 MIDI 轨和 GM 鼓音高，排出一段两小节的基础节奏。",
    "steps": [
      {
        "id": "new-track",
        "title": "新建一条 MIDI 轨当鼓轨",
        "body": "夜半没有单独的鼓轨类型：鼓就是一条 MIDI 轨，音高对到 GM 鼓音色区。从「轨道」菜单选「+ 新建 MIDI 合成器轨」（Ctrl+N），再给它装一份带鼓组的音色。",
        "check": { "kind": "new_track", "track_kind": "midi" }
      },
      {
        "id": "kick-snare",
        "title": "排出底鼓与军鼓",
        "body": "打开钢琴卷帘（P），在 GM 鼓音高上落点：36 是底鼓、38 是军鼓。半小节内写至少 8 个音、至少 2 种音高——比如底鼓踩一、三拍，军鼓踩二、四拍。",
        "check": { "kind": "notes_on_new_track", "min_notes": 8, "min_distinct_pitches": 2, "min_span_ticks": 1920, "pitch_low": 35, "pitch_high": 81 }
      },
      {
        "id": "hats-two-bars",
        "title": "加上踩镲，写满两小节",
        "body": "42 是闭合踩镲。把节奏写满 2 小节：至少 16 个音、3 种音高。写完按 Space 播放，鼓组应该能自己循环成一段律动。",
        "check": { "kind": "notes_on_new_track", "min_notes": 16, "min_distinct_pitches": 3, "min_span_ticks": 7680, "pitch_low": 35, "pitch_high": 81 }
      }
    ]
  }
]"#;

/// 全部内置课程。解析失败是编程错误（字面量写错），响亮 panic，
/// 判据在 CI 里会先一步抓到。
#[must_use]
pub fn catalog() -> Vec<Lesson> {
    serde_json::from_str(CATALOG_JSON).expect("内置课程表 JSON 必须合法")
}

/// 按 id 取一节课。
#[must_use]
pub fn lesson(id: &str) -> Option<Lesson> {
    catalog().into_iter().find(|lesson| lesson.id == id)
}
