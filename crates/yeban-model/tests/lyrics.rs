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

use yeban_model::lyrics::{LyricLine, LyricParseError, lyrics_to_lrc, parse_lrc, validate_lyrics};
use yeban_model::samples::{default_project, filled_project};
use yeban_model::{ModelError, YebanProjectV1};

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
