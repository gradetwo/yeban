//! 歌词轨的判据：存取、LRC 编解码、往返一致、旧工程不变。
//!
//! ## 这条判据要证的句子
//!
//! > 工程存得下“一行词在几时唱”，导出的 LRC 与 Wang 的参照 LRC（`winter.lrc`）
//! > 同一口径，旧工程（无词）逐字节不动。
//!
//! ## 口径锚点
//!
//! 参照 LRC 的前 5 行（`winter.lrc`，132 BPM、每行隔 8 拍 = 7680 tick）：
//!
//! ```text
//! [00:00.00]凛冬将至
//! [00:03.64]Winter Is Coming
//! [00:07.27]凛冬将至
//! [00:10.91]Winter Is Coming
//! [00:14.55]昨日 你 信誓旦旦
//! ```
//!
//! 本文件用同一组 tick（0 / 7680 / 15360 / 23040 / 30720 @132 BPM）钉住导出格式。
//! 唯一**刻意**的差异：参照有 `[al:Winter Is Coming]`，模型里没有专辑字段 ⇒
//! 导出不写 `al`（不编造）；其余头标签（`ti`/`ar`/`by`/`length`/`offset`）逐项对过。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_model::lyrics::{
    LyricLine, LyricParseError, converged_lyric_lines, lyrics_to_lrc, lyrics_to_lrc_converged,
    parse_lrc, validate_lyrics,
};
use yeban_model::samples::{default_project, filled_project};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, ModelError, TrackV3,
    YebanProjectV1,
};

/// 汇聚夹具的 ULID：`01J8ZM` + 20 位十进制序号。
fn converge_id(index: u32) -> EntityId {
    EntityId::from_str(&format!("01J8ZM{index:020}")).expect("夹具 ULID 必须合法")
}

/// 仅音节工程：一条轨、一个摆放、片段音符带 `syllable`，时间轴为空。
///
/// 音符表是 `(起始 tick, 时值, syllable)`；摆放起点由参数给。
fn syllable_project(notes: &[(u64, u64, Option<&str>)], placement_start: u64) -> YebanProjectV1 {
    let mut project = default_project();
    let track_id = converge_id(2);
    let clip_id = converge_id(10);
    let mut pool = BTreeMap::new();
    for (index, &(start_tick, duration_ticks, syllable)) in notes.iter().enumerate() {
        let note_id = converge_id(100 + u32::try_from(index).expect("下标"));
        let mut note = MidiNote::new(note_id, start_tick, 60, duration_ticks);
        note.syllable = syllable.map(ToOwned::to_owned);
        pool.insert(note_id, note);
    }
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "Vocal".to_owned(),
            content: ClipContent::Midi { notes: pool },
        },
    );
    let placement_id = converge_id(50);
    let mut track = TrackV3 {
        id: track_id,
        name: "Lead".to_owned(),
        ..TrackV3::default()
    };
    track.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: placement_start,
            duration_ticks: 15_360,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    project.tracks.insert(track_id, track);
    project.validate().expect("汇聚夹具合法");
    project
}

/// 参照口径的夹具：132 BPM，5 行词，每行隔 8 拍（7680 tick）。
fn winter_fixture() -> YebanProjectV1 {
    let mut project = default_project();
    project.title = "凛冬将至".to_owned();
    project.author = "夜半 · Yeban".to_owned();
    project.bpm = 132.0;
    project.lyrics = vec![
        LyricLine {
            start_tick: 0,
            text: "凛冬将至".to_owned(),
        },
        LyricLine {
            start_tick: 7680,
            text: "Winter Is Coming".to_owned(),
        },
        LyricLine {
            start_tick: 15360,
            text: "凛冬将至".to_owned(),
        },
        LyricLine {
            start_tick: 23040,
            text: "Winter Is Coming".to_owned(),
        },
        LyricLine {
            start_tick: 30720,
            text: "昨日 你 信誓旦旦".to_owned(),
        },
    ];
    project
}

#[test]
fn empty_lyrics_are_not_written_out() {
    // 空表不写出（与 `track_order` 同一条 skip discipline）：旧工程逐字节不动。
    let project = default_project();
    assert!(project.lyrics.is_empty());
    let value = serde_json::to_value(&project).expect("序列化");
    assert!(
        value.get("lyrics").is_none(),
        "无词工程不得写出 `lyrics` 键: {value}"
    );
    let filled = serde_json::to_value(filled_project()).expect("序列化夹具");
    assert!(
        filled.get("lyrics").is_none(),
        "冻结夹具不得写出 `lyrics` 键"
    );
    let back: YebanProjectV1 = serde_json::from_value(value).expect("反序列化");
    assert_eq!(back, project);
}

#[test]
fn lyrics_round_trip_through_json() {
    let project = winter_fixture();
    let value = serde_json::to_value(&project).expect("序列化");
    let lyrics = value.get("lyrics").expect("有词工程必须写出 `lyrics` 键");
    assert_eq!(lyrics.as_array().expect("数组").len(), 5);
    assert_eq!(lyrics[0]["start_tick"], 0);
    assert_eq!(lyrics[0]["text"], "凛冬将至");
    assert_eq!(lyrics[4]["start_tick"], 30720);
    assert_eq!(lyrics[4]["text"], "昨日 你 信誓旦旦");
    let back: YebanProjectV1 = serde_json::from_value(value).expect("反序列化");
    assert_eq!(back, project);
    back.validate().expect("夹具合法");
}

#[test]
fn lyrics_export_matches_the_reference_format() {
    let project = winter_fixture();
    let lrc = lyrics_to_lrc(&project, None).expect("导出");
    assert_eq!(
        lrc,
        "[ti:凛冬将至]\n[ar:夜半 · Yeban]\n[by:Yeban]\n[offset:0]\n\n\
         [00:00.00]凛冬将至\n[00:03.64]Winter Is Coming\n[00:07.27]凛冬将至\n\
         [00:10.91]Winter Is Coming\n[00:14.55]昨日 你 信誓旦旦\n"
    );
    // 给了总时长才写 `[length:]`（参照 `winter.lrc` 的 03:23.63）。
    let with_length = lyrics_to_lrc(&project, Some(203.63)).expect("导出");
    assert!(
        with_length.contains("[length:03:23.63]\n"),
        "总时长应写成 [length:03:23.63]:\n{with_length}"
    );
    // 总时长非法 ⇒ 响亮失败，⛔ 不写一个假长度（NaN 不自等于 NaN，用 matches 判变体）。
    assert!(
        lyrics_to_lrc(&project, Some(f64::NAN)).is_err_and(|error| matches!(
            error,
            ModelError::NonFiniteValue {
                field: "lyrics.total_seconds",
                ..
            }
        )),
        "NaN 总时长必须失败"
    );
}

#[test]
fn old_project_without_lyrics_exports_header_only() {
    let project = default_project();
    let lrc = lyrics_to_lrc(&project, None).expect("导出");
    assert_eq!(lrc, "[ti:Untitled]\n[by:Yeban]\n[offset:0]\n\n");
}

#[test]
fn reference_lrc_excerpt_parses_to_the_same_lines() {
    // 参照 `winter.lrc` 的头 + 前 5 行（`[al:]` 行在导入时忽略 —— 模型里没有专辑字段）。
    let text = "[ti:凛冬将至]\n[ar:夜半 · Yeban]\n[al:Winter Is Coming]\n[by:夜半 MCP]\n\
                [length:03:23.63]\n[offset:0]\n\n[00:00.00]凛冬将至\n[00:03.64]Winter Is Coming\n\
                [00:07.27]凛冬将至\n[00:10.91]Winter Is Coming\n[00:14.55]昨日 你 信誓旦旦\n";
    let project = winter_fixture();
    let lines = parse_lrc(text, &project.tempo(), 48_000).expect("解析参照 LRC");
    assert_eq!(lines.len(), 5);
    let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "凛冬将至",
            "Winter Is Coming",
            "凛冬将至",
            "Winter Is Coming",
            "昨日 你 信誓旦旦"
        ]
    );
    // 厘秒取整会让 tick 漂移：132 BPM 下 1 厘秒 = 21.12 tick，允差 22 tick。
    let nominal = [0_u64, 7680, 15360, 23040, 30720];
    for (line, want) in lines.iter().zip(nominal) {
        let drift = line.start_tick.abs_diff(want);
        assert!(
            drift <= 22,
            "第 {} 行 tick {} 偏离标称 {} 超过 1 厘秒（{} tick）",
            line.text,
            line.start_tick,
            want,
            drift
        );
    }
}

#[test]
fn lyrics_round_trip_is_byte_identical() {
    let project = winter_fixture();
    let lrc = lyrics_to_lrc(&project, None).expect("导出");
    let lines = parse_lrc(&lrc, &project.tempo(), 48_000).expect("解析自家导出");
    let mut again = winter_fixture();
    again.lyrics = lines;
    let lrc_again = lyrics_to_lrc(&again, None).expect("再导出");
    assert_eq!(lrc_again, lrc, "导出 ⇒ 解析 ⇒ 再导出必须逐字节相同");
}

#[test]
fn validate_rejects_bad_lyrics() {
    assert_eq!(
        validate_lyrics(&[LyricLine {
            start_tick: 0,
            text: String::new(),
        }]),
        Err(ModelError::LyricTextEmpty { start_tick: 0 })
    );
    assert_eq!(
        validate_lyrics(&[
            LyricLine {
                start_tick: 100,
                text: "甲".to_owned(),
            },
            LyricLine {
                start_tick: 100,
                text: "乙".to_owned(),
            },
        ]),
        Err(ModelError::LyricStartNotAscending {
            previous: 100,
            start_tick: 100,
        })
    );
    assert_eq!(
        validate_lyrics(&[
            LyricLine {
                start_tick: 200,
                text: "甲".to_owned(),
            },
            LyricLine {
                start_tick: 100,
                text: "乙".to_owned(),
            },
        ]),
        Err(ModelError::LyricStartNotAscending {
            previous: 200,
            start_tick: 100,
        })
    );
    let mut project = winter_fixture();
    project.validate().expect("夹具合法");
    project.lyrics[1].text.clear();
    assert_eq!(
        project.validate(),
        Err(ModelError::LyricTextEmpty { start_tick: 7680 })
    );
}

#[test]
fn parse_errors_are_loud() {
    let project = winter_fixture();
    let tempo = project.tempo();
    // 秒 = 60 不是合法时钟读数。
    assert_eq!(
        parse_lrc("[00:60.00]词\n", &tempo, 48_000),
        Err(LyricParseError::BadTimestamp {
            line: 1,
            text: "[00:60.00]词".to_owned(),
        })
    );
    // 厘秒不是 2 位。
    assert_eq!(
        parse_lrc("[00:03.6]词\n", &tempo, 48_000),
        Err(LyricParseError::BadTimestamp {
            line: 1,
            text: "[00:03.6]词".to_owned(),
        })
    );
    // 没有时间戳的词行没有落点。
    assert_eq!(
        parse_lrc("没有时间戳的一行\n", &tempo, 48_000),
        Err(LyricParseError::LineWithoutTimestamp {
            line: 1,
            text: "没有时间戳的一行".to_owned(),
        })
    );
    // offset 把时间戳移成负数。
    assert_eq!(
        parse_lrc("[offset:-5000]\n[00:03.00]词\n", &tempo, 48_000),
        Err(LyricParseError::NegativeTimestamp {
            line: 2,
            offset_ms: -5000,
        })
    );
    // offset 正常移位：+1000 毫秒 ⇒ 第一行落在 1 秒处（132 BPM 下 2112 tick）。
    let shifted = parse_lrc("[offset:1000]\n[00:00.00]词\n", &tempo, 48_000).expect("解析");
    assert_eq!(shifted.len(), 1);
    assert_eq!(shifted[0].start_tick, 2112);
    // 一行多个时间戳 ⇒ 同一行词在多处唱；未知头标签忽略。
    let multi = parse_lrc("[ti:标题]\n[00:00.00][00:03.64]副歌\n", &tempo, 48_000).expect("解析");
    assert_eq!(multi.len(), 2);
    assert_eq!(multi[0].text, "副歌");
    assert_eq!(multi[1].text, "副歌");
    assert!(multi[0].start_tick < multi[1].start_tick);
}

#[test]
fn syllable_only_project_converges_to_lines() {
    // 相邻音节间隔 ≤ 一拍合成一行；间隔超过一拍断行（设计 §2.3）。
    let project = syllable_project(
        &[
            (0, 480, Some("凛")),
            (960, 480, Some("冬")),
            (7_680, 480, Some("至")),
        ],
        0,
    );
    assert!(project.lyrics.is_empty(), "夹具时间轴必须为空");
    let lines = converged_lyric_lines(&project);
    assert_eq!(
        lines,
        vec![
            LyricLine {
                start_tick: 0,
                text: "凛 冬".to_owned(),
            },
            LyricLine {
                start_tick: 7_680,
                text: "至".to_owned(),
            },
        ]
    );
    // 旧入口仍只读时间轴（逐字节头-only）；汇聚入口给出完整歌词。
    let plain = lyrics_to_lrc(&project, None).expect("旧入口导出");
    assert!(!plain.contains("凛"), "旧入口不得汇聚: {plain}");
    let converged = lyrics_to_lrc_converged(&project, None).expect("汇聚导出");
    assert!(
        converged.contains("凛 冬"),
        "汇聚导出必须含首行: {converged}"
    );
    assert!(converged.contains("至"), "汇聚导出必须含次行: {converged}");
}

#[test]
fn timeline_wins_at_the_same_tick_and_other_syllables_survive() {
    let mut project = syllable_project(
        &[
            (0, 480, Some("凛")),
            (960, 480, Some("冬")),
            (7_680, 480, Some("至")),
        ],
        0,
    );
    project.lyrics = vec![
        LyricLine {
            start_tick: 0,
            text: "时间轴首行".to_owned(),
        },
        LyricLine {
            start_tick: 15_360,
            text: "时间轴尾行".to_owned(),
        },
    ];
    project.validate().expect("夹具合法");
    // tick 0 的汇聚行被时间轴行取代；tick 7680 的汇聚行保留并按序合并。
    assert_eq!(
        converged_lyric_lines(&project),
        vec![
            LyricLine {
                start_tick: 0,
                text: "时间轴首行".to_owned(),
            },
            LyricLine {
                start_tick: 7_680,
                text: "至".to_owned(),
            },
            LyricLine {
                start_tick: 15_360,
                text: "时间轴尾行".to_owned(),
            },
        ]
    );
}

#[test]
fn empty_syllables_produce_no_lines() {
    let project = syllable_project(
        &[
            (0, 480, None),
            (960, 480, Some("")),
            (1_920, 480, Some("  ")),
            (3_840, 480, Some("唱")),
        ],
        0,
    );
    assert_eq!(
        converged_lyric_lines(&project),
        vec![LyricLine {
            start_tick: 3_840,
            text: "唱".to_owned(),
        }],
        "空音节不得产生空行，也不得挡住后面的真音节"
    );
    let silent = syllable_project(&[(0, 480, Some("")), (960, 480, None)], 0);
    assert!(converged_lyric_lines(&silent).is_empty());
    let lrc = lyrics_to_lrc_converged(&silent, None).expect("汇聚导出");
    assert_eq!(lrc, "[ti:Untitled]\n[by:Yeban]\n[offset:0]\n\n");
}

#[test]
fn muted_placements_and_unplaced_clips_do_not_converge() {
    let mut project = syllable_project(&[(0, 480, Some("唱"))], 0);
    // 同一轨再加一个静音摆放，指向另一片段的音节。
    let ghost_clip = converge_id(11);
    let ghost_note = converge_id(200);
    let mut ghost = MidiNote::new(ghost_note, 0, 60, 480);
    ghost.syllable = Some("不唱".to_owned());
    project.clip_pool.insert(
        ghost_clip,
        ClipPoolEntry {
            id: ghost_clip,
            name: "Ghost".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(ghost_note, ghost)]),
            },
        },
    );
    // 未被摆放引用的片段同样不汇聚：再建一个片段但不摆放。
    let unplaced_clip = converge_id(12);
    let unplaced_note = converge_id(201);
    let mut unplaced = MidiNote::new(unplaced_note, 0, 60, 480);
    unplaced.syllable = Some("没摆".to_owned());
    project.clip_pool.insert(
        unplaced_clip,
        ClipPoolEntry {
            id: unplaced_clip,
            name: "Unplaced".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(unplaced_note, unplaced)]),
            },
        },
    );
    let track_id = converge_id(2);
    let placement_id = converge_id(51);
    project
        .tracks
        .get_mut(&track_id)
        .expect("主轨")
        .clips
        .insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id: ghost_clip,
                start_tick: 7_680,
                duration_ticks: 960,
                loop_config: LoopConfig::default(),
                muted: true,
                offset_samples: None,
            },
        );
    project.validate().expect("夹具合法");
    assert_eq!(
        converged_lyric_lines(&project),
        vec![LyricLine {
            start_tick: 0,
            text: "唱".to_owned(),
        }]
    );
}
