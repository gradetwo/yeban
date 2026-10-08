//! `yeban-midi` 在**真 SMF 文件**上的"接受"判据。
//!
//! ## 这个文件补的是哪个缺口（先量后做）
//!
//! `tests/smf_contract.rs`（提交 `be04401`）钉住了**拒绝**路径与手工拼字节的契约。
//! 但本 crate 在本次改动前**没有**任何真文件夹具：全部输入都是本仓库自己编出的字节，
//! 或手工拼的规范样本。`tests/smf_contract.rs:24-29` 只声明了 `Encode` 与
//! SMPTE/格式 2 的未证明项，没有声明"真文件零覆盖"这个洞。
//!
//! 于是有一条承诺从未被真文件检验：**一个合法真文件必须被完整接受**，
//! 而不是被静默丢掉轨道、音符或 tempo。
//!
//! 因此本文件的判据是：把三个**公有领域**的真文件读进来，
//! 并把**实测**出来的轨道数 / 音符数 / tick 范围 / tempo / 力度钉死，
//! 再让编码器面对真世界的音符形状跑一次端到端往返。
//!
//! ## 夹具
//!
//! `tests/fixtures/` 下三个文件（贝多芬《致爱丽丝》，公有领域；见该目录 `README.md`）：
//!
//! | 常量 | 夹具 | 字节 | 源文件 |
//! | :--- | :--- | ---: | :--- |
//! | [`WOO59`] | `fur_elise_woo59_384ppq_3mtrk.mid` | 7590 | `/tmp/midi/fur_Elise_WoO59.mid` |
//! | [`ELISE_1`] | `fur_elise_480ppq_1mtrk.mid` | 3822 | `/tmp/midi/Für Elise 1 tracks.mid` |
//! | [`ELISE_2`] | `fur_elise_480ppq_3mtrk.mid` | 6687 | `/tmp/midi/Für Elise 2 tracks.mid` |
//!
//! `include_bytes!` 相对本文件所在目录解析 ⇒ 判据不依赖运行时工作目录。
//!
//! ## 本票在真素材上实测到的一条**缺陷**（判据 ⑦登记，本票**不**修）
//!
//! 真文件的 tempo map 在 `parse_smf` → `to_smf_bytes` → `parse_smf` 之后**不保真**：
//! 当**两条 tempo 事件落在同一 tick** 时，拍号与 tempo 的配对会丢失，
//! 且 `parse_smf` 为"孤儿拍号"合成的默认 tempo 会被**写进文件**。
//! 实测的导线级证据（量法：`/tmp` 探针用 `midly::Smf::parse` 枚举
//! `MetaMessage::Tempo` / `TimeSignature`，按 `(tick, 值)` 归一后比多重集）：
//!
//! ```text
//! 原始   : ["tick=0 TEMPO 833333", "tick=0 TIMESIG 3/3", "tick=0 TIMESIG 4/2"]        (3 条)
//! 再导出 : ["tick=0 TEMPO 500000", "tick=0 TEMPO 833333", "tick=0 TIMESIG 3/3",
//!           "tick=0 TIMESIG 4/2"]                                                     (4 条)
//! ```
//!
//! 量法（`/tmp` 探针）：数 `ParsedMidi.tempos` 里 `tick == 0` 的条目 ≥ 2 的文件个数。
//! 读数：**9 / 16** 个真文件具备"同一 tick 两条 tempo"这个形状。
//! 修它要改 `MidiTempo` 的语义或 `parse_smf` 的配对规则 ⇒ 那是**跨 crate**的语义裁决，
//! 不在本票范围。判据 ⑦把当前行为钉成**字面读数**：日后有人修它，这条会**故意**变红。
//!
//! ## 本文件**没有**证明什么
//!
//! - 只覆盖格式 0（`SingleTrack`）与格式 1（`Parallel`）、PPQ 384 与 PPQ 480。
//!   格式 2 与 SMPTE 时间码仍然只证明"被拒绝"。
//! - 力度在这三个夹具里**全部相同**（实测 62）。因此力度往返没有被真文件覆盖。
//! - `MidiError::UnclosedNote` / `UnmatchedNoteOff` 没有被真文件触发。
//!   本机对 16 个真文件全部解析成功（探针读数），没有找到能触发它们的真文件。
//! - tempo map 的往返保真**不成立**（见上一节）。判据 ⑦只登记，不承诺。

use std::collections::BTreeMap;

use yeban_midi::midi::{
    MidiError, MidiExport, MidiExportTrack, MidiFormat, MidiTempo, ParsedMidi, parse_smf,
    track_chunks,
};
use yeban_model::EntityId;
use yeban_model::music::MidiNote;

/// 贝多芬《致爱丽丝》`WoO 59`：格式 1、PPQ 384、3 条 `MTrk`、905 颗音符。
const WOO59: &[u8] = include_bytes!("fixtures/fur_elise_woo59_384ppq_3mtrk.mid");
/// 同一曲目的 PPQ 480 单轨版本：格式 0、1 条 `MTrk`、517 颗音符。
const ELISE_1: &[u8] = include_bytes!("fixtures/fur_elise_480ppq_1mtrk.mid");
/// 同一曲目的 PPQ 480 双声部版本：格式 1、3 条 `MTrk`、905 颗音符。
const ELISE_2: &[u8] = include_bytes!("fixtures/fur_elise_480ppq_3mtrk.mid");

/// 实测读数：一个真文件被接受后，我们关心的全部数字。
#[derive(Debug, PartialEq, Eq)]
struct Reading {
    /// 文件字节数。
    bytes: usize,
    /// `track_chunks` 返回的 chunk 总数（含 `MThd`）。
    chunks: usize,
    /// `fourcc == MTrk` 的 chunk 数。
    mtrk: usize,
    /// `MThd` 里的格式号。
    format: MidiFormat,
    /// `MThd` 里的时间分度。
    ppq: u16,
    /// 音符总数。
    notes: usize,
    /// 最小的音符起始 tick（无音符时 `None`）。
    min_start_tick: Option<u64>,
    /// 最大的音符结束 tick（无音符时 `None`）。
    max_end_tick: Option<u64>,
    /// 每个通道的音符数（`BTreeMap` ⇒ 键序确定）。
    per_channel: BTreeMap<u8, usize>,
    /// tempo map。
    tempos: Vec<MidiTempo>,
    /// 不同的音高数。
    distinct_pitches: usize,
    /// 力度直方图。
    velocities: BTreeMap<u8, usize>,
}

/// 把一个真文件的字节读成 [`Reading`]。
///
/// 这里用 `expect` 是**判据**的一部分：夹具在编译期由 `include_bytes!` 固定，
/// 解析失败就是真的回归，必须让测试停在这里而不是继续。
fn read(bytes: &[u8]) -> Reading {
    let chunks = track_chunks(bytes).expect("夹具的 chunk 布局必须合法");
    let parsed = parse_smf(bytes).expect("夹具必须被接受");
    let mut per_channel: BTreeMap<u8, usize> = BTreeMap::new();
    let mut velocities: BTreeMap<u8, usize> = BTreeMap::new();
    let mut pitches: Vec<u8> = Vec::new();
    for note in &parsed.notes {
        *per_channel.entry(note.channel).or_default() += 1;
        *velocities.entry(note.velocity).or_default() += 1;
        pitches.push(note.key);
    }
    pitches.sort_unstable();
    pitches.dedup();
    Reading {
        bytes: bytes.len(),
        chunks: chunks.len(),
        mtrk: chunks.iter().filter(|c| &c.fourcc == b"MTrk").count(),
        format: parsed.format,
        ppq: parsed.ppq,
        notes: parsed.notes.len(),
        min_start_tick: parsed.notes.iter().map(|n| n.start_tick).min(),
        max_end_tick: parsed.notes.iter().map(|n| n.end_tick).max(),
        per_channel,
        tempos: parsed.tempos.clone(),
        distinct_pitches: pitches.len(),
        velocities,
    }
}

/// 一个真文件里全部音符的规范化键（`(channel, key, velocity, start, duration)`）。
///
/// 排序后比较 ⇒ 这是**多重集**相等，而不是"顺序碰巧一样"。
fn sorted_keys(parsed: &ParsedMidi) -> Vec<(u8, u8, u8, u64, u64)> {
    let mut keys: Vec<_> = parsed.notes.iter().map(|n| n.key()).collect();
    keys.sort_unstable();
    keys
}

/// 把回读结果重新装成一次导出（真文件 → `MidiExport`）。
///
/// 力度从回读结果里搬过来，因为 `MidiNote::new` 会把力度固定成默认值。
fn reexport(parsed: &ParsedMidi, format: MidiFormat) -> MidiExport {
    let mut by_channel: BTreeMap<u8, Vec<MidiNote>> = BTreeMap::new();
    for note in &parsed.notes {
        let mut model = MidiNote::new(
            EntityId::new(),
            note.start_tick,
            note.key,
            note.duration_ticks(),
        );
        model.velocity = note.velocity;
        by_channel.entry(note.channel).or_default().push(model);
    }
    MidiExport {
        format,
        ppq: parsed.ppq,
        tempos: parsed.tempos.clone(),
        tracks: by_channel
            .into_iter()
            .map(|(channel, notes)| MidiExportTrack {
                name: format!("channel {channel}"),
                channel,
                notes,
            })
            .collect(),
    }
}

/// 判据 ③的核心谓词：双声部文件相对单声部文件的差值必须是 `(2 条 MTrk, 388 颗音符)`。
///
/// 判据 ⑥用它证明这条判据**有牙齿**：同一个谓词在丢掉一条轨道后必须为 `false`。
fn channel_one_diff_holds(one: &Reading, three: &Reading) -> bool {
    three.mtrk.saturating_sub(one.mtrk) == 2 && three.notes.saturating_sub(one.notes) == 388
}

/// 判据 ①: 公有领域的真文件 `WoO 59` 被完整接受，全部读数是**实测值**。
#[test]
fn woo59_real_file_is_accepted_with_measured_readings() {
    let got = read(WOO59);
    let want = Reading {
        bytes: 7590,
        chunks: 4, // 1 × MThd + 3 × MTrk
        mtrk: 3,
        format: MidiFormat::Parallel,
        ppq: 384,
        notes: 905,
        min_start_tick: Some(0),
        max_end_tick: Some(60_096),
        per_channel: BTreeMap::from([(0u8, 517usize), (1, 388)]),
        // 第 0 条 MTrk 是 conductor：3/8 拍 + 500000 µs/四分音符。
        // 另一条 833333 µs 的 tempo 在**同一 tick**上，且没有配对的拍号事件。
        tempos: vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: 500_000,
                numerator: Some(3),
                denominator_pow2: Some(3),
            },
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: 833_333,
                numerator: None,
                denominator_pow2: None,
            },
        ],
        distinct_pitches: 56,
        velocities: BTreeMap::from([(62u8, 905usize)]),
    };
    assert_eq!(got, want, "WoO 59 的真文件读数与实测不符");
}

/// 判据 ②: 两个 PPQ 480 的真文件被完整接受（一个格式 0、一个格式 1）。
#[test]
fn fur_elise_480ppq_real_files_are_accepted_with_measured_readings() {
    let one = read(ELISE_1);
    assert_eq!(
        one,
        Reading {
            bytes: 3822,
            chunks: 2, // 1 × MThd + 1 × MTrk
            mtrk: 1,
            format: MidiFormat::SingleTrack,
            ppq: 480,
            notes: 517,
            min_start_tick: Some(0),
            max_end_tick: Some(75_120),
            per_channel: BTreeMap::from([(0u8, 517usize)]),
            // 格式 0 把 tempo 事件与音符塞进同一条 MTrk。拍号在 tempo 的同一 tick 上。
            tempos: vec![
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: 500_000,
                    numerator: Some(3),
                    denominator_pow2: Some(3),
                },
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: 833_333,
                    numerator: Some(4),
                    denominator_pow2: Some(2),
                },
            ],
            distinct_pitches: 39,
            velocities: BTreeMap::from([(62u8, 517usize)]),
        },
        "PPQ 480 单 MTrk 的真文件读数与实测不符"
    );

    let three = read(ELISE_2);
    assert_eq!(
        three,
        Reading {
            bytes: 6687,
            chunks: 4, // 1 × MThd + 3 × MTrk
            mtrk: 3,
            format: MidiFormat::Parallel,
            ppq: 480,
            notes: 905,
            min_start_tick: Some(0),
            max_end_tick: Some(75_120),
            per_channel: BTreeMap::from([(0u8, 517usize), (1, 388)]),
            tempos: vec![
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: 500_000,
                    numerator: Some(3),
                    denominator_pow2: Some(3),
                },
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: 833_333,
                    numerator: Some(4),
                    denominator_pow2: Some(2),
                },
            ],
            distinct_pitches: 56,
            velocities: BTreeMap::from([(62u8, 905usize)]),
        },
        "PPQ 480 三 MTrk 的真文件读数与实测不符"
    );
}

/// 判据 ③: 两个 PPQ 480 真文件的差异**就是**通道 1 —— 逐项算术。
///
/// 这条判据的牙齿在"解析器丢掉一条轨道"或"音符数算错"时咬合。
/// 注入证明见判据 ⑥。
#[test]
fn the_two_fur_elise_480ppq_files_differ_by_exactly_channel_one() {
    let one = read(ELISE_1);
    let three = read(ELISE_2);
    let one_parsed = parse_smf(ELISE_1).expect("接受");
    let three_parsed = parse_smf(ELISE_2).expect("接受");

    assert!(
        channel_one_diff_holds(&one, &three),
        "差值必须是 (2 条 MTrk, 388 颗音符)，实际 ({}, {})",
        three.mtrk.saturating_sub(one.mtrk),
        three.notes.saturating_sub(one.notes)
    );

    // 结构差：多出 2 条 MTrk（conductor + 通道 1），格式从 0 变 1。
    assert_eq!(three.mtrk - one.mtrk, 2, "MTrk 数差 = 2");
    assert_eq!(three.chunks - one.chunks, 2, "chunk 数差 = 2");
    assert_eq!(one.format, MidiFormat::SingleTrack);
    assert_eq!(three.format, MidiFormat::Parallel);

    // 字节差（两个数由 `stat -f%z` 量出）。
    assert_eq!(three.bytes - one.bytes, 2865, "6687 - 3822 = 2865");

    // 音符差：388 颗，全部落在通道 1。
    assert_eq!(three.notes - one.notes, 388, "905 - 517 = 388");
    assert_eq!(
        three.per_channel.get(&1),
        Some(&388),
        "多出的音符全在通道 1"
    );
    assert_eq!(
        one.per_channel.get(&0),
        three.per_channel.get(&0),
        "两个文件的通道 0 音符数相同"
    );

    // 通道 0 的音符序列**逐键相同** ⇒ 单 MTrk 版本就是通道 0 那一份。
    let only_channel_0: Vec<_> = sorted_keys(&three_parsed)
        .into_iter()
        .filter(|key| key.0 == 0)
        .collect();
    assert_eq!(
        only_channel_0,
        sorted_keys(&one_parsed),
        "双声部文件的通道 0 必须与单声部文件逐音符相同"
    );

    // 时间跨度**没有**变：多出的 388 颗音符落在既有跨度之内，不是追加在后面。
    assert_eq!(one.min_start_tick, three.min_start_tick, "起点 tick 相同");
    assert_eq!(one.max_end_tick, three.max_end_tick, "终点 tick 相同");

    // tempo map 逐字段相同。
    assert_eq!(one.tempos, three.tempos, "tempo map 相同");

    // PPQ 相同 ⇒ "音符数不同"不可能由重采样解释。
    assert_eq!(one.ppq, three.ppq, "两者的 PPQ 都是 480");

    // 音域差：多出的低音把不同音高数从 39 拉到 56。
    assert_eq!(
        three.distinct_pitches - one.distinct_pitches,
        17,
        "56 - 39 = 17"
    );
}

/// 判据 ④: `WoO 59`（PPQ 384）与 PPQ 480 双声部版本携带**同一份音符**，tick 按比例缩放。
#[test]
fn woo59_and_480ppq_versions_carry_the_same_notes_at_scaled_ticks() {
    let slow = read(WOO59);
    let fast = read(ELISE_2);

    assert_eq!(slow.notes, fast.notes, "音符数相同：905");
    assert_eq!(slow.per_channel, fast.per_channel, "逐通道音符数相同");
    assert_eq!(slow.velocities, fast.velocities, "力度直方图相同");
    assert_eq!(
        slow.distinct_pitches, fast.distinct_pitches,
        "不同音高数相同"
    );
    assert_eq!(slow.min_start_tick, fast.min_start_tick, "起点 tick 都是 0");

    // PPQ 差 384 → 480（比 5:4）⇒ 终点 tick 严格按 5:4 缩放。
    assert_eq!(slow.ppq, 384);
    assert_eq!(fast.ppq, 480);
    assert_eq!(slow.max_end_tick, Some(60_096));
    assert_eq!(fast.max_end_tick, Some(75_120));
    assert_eq!(
        60_096u64 * u64::from(fast.ppq),
        75_120u64 * u64::from(slow.ppq),
        "60096 × 480 == 75120 × 384（两个文件表示同一段音乐）"
    );

    // tempo 的 (tick, µs/四分音符) 序列相同。故意**不**比较拍号：
    // 480 版本的第 2 条 tempo 带 4/4，384 版本那条不带（见判据 ①②的实测值）。
    assert_eq!(
        slow.tempos
            .iter()
            .map(|t| (t.tick, t.microseconds_per_quarter))
            .collect::<Vec<_>>(),
        fast.tempos
            .iter()
            .map(|t| (t.tick, t.microseconds_per_quarter))
            .collect::<Vec<_>>(),
        "tempo 的 (tick, µs/四分音符) 序列相同"
    );
}

/// 判据 ⑤: 真文件 → `MidiExport` → `to_smf_bytes` → `parse_smf` 的**音符**不变。
///
/// 这是本文件唯一的**端到端**判据：它让编码器面对真世界的音符形状
/// （905 颗音符、两个通道、音高 33..100、tick 到 75120），而不是手工造的样本。
///
/// 只断言 PPQ 与音符。tempo map **不**在此断言 —— 它不保真，见判据 ⑦。
#[test]
fn public_domain_files_survive_an_export_readback_round_trip() {
    let cases: [(&str, &[u8], MidiFormat); 3] = [
        ("WoO 59 (384 PPQ)", WOO59, MidiFormat::Parallel),
        (
            "Für Elise 1 MTrk (480 PPQ)",
            ELISE_1,
            MidiFormat::SingleTrack,
        ),
        ("Für Elise 3 MTrk (480 PPQ)", ELISE_2, MidiFormat::Parallel),
    ];
    for (label, bytes, format) in cases {
        let first = parse_smf(bytes).expect("夹具必须被接受");
        let export = reexport(&first, format);
        let reemitted = export.to_smf_bytes().expect("真音符必须能编码");
        let second = parse_smf(&reemitted).expect("自己编出的字节必须能读回");

        assert_eq!(second.ppq, first.ppq, "{label}: PPQ 必须保留");
        assert_eq!(
            second.notes.len(),
            first.notes.len(),
            "{label}: 音符数必须不变"
        );
        assert_eq!(
            sorted_keys(&second),
            sorted_keys(&first),
            "{label}: 往返后的音符多重集必须不变"
        );
        // 通道跨度也必须不变，否则"音符数相同"可能是两条轨道被合并。
        assert_eq!(
            second
                .notes
                .iter()
                .map(|n| n.channel)
                .collect::<std::collections::BTreeSet<_>>(),
            first
                .notes
                .iter()
                .map(|n| n.channel)
                .collect::<std::collections::BTreeSet<_>>(),
            "{label}: 通道集合必须不变"
        );
    }
}

/// 判据 ⑥: 判据 ③的谓词**有牙齿** —— 丢掉一条真轨道就会变红。
///
/// 这里用**同一份真夹具的字节**构造两个注入，并给出字面读数：
///
/// 1. 只改 `MThd` 的 `ntracks`（偏移 `11`，`3 → 2`）⇒ 读数**完全不变**。
///    实测原因：`midly` 与 `track_chunks` 都**不读** `ntracks`，而是读到文件末尾。
///    所以 `ntracks` 在本 crate 里不是承重字段。这一点必须登记，不能假设。
/// 2. 真正截掉最后一条 `MTrk` chunk ⇒ `MTrk` 数 `3 → 2`，音符数 `905 → 517`，
///    判据 ③的谓词从 `true` 变 `false`。
#[test]
fn the_track_count_assertion_has_teeth_when_a_real_track_is_dropped() {
    let intact = read(ELISE_2);
    let one = read(ELISE_1);
    assert!(
        channel_one_diff_holds(&one, &intact),
        "前提：夹具上谓词为真"
    );

    // ---- 注入 1：只改 MThd 的 ntracks ⇒ 读数不变（实测的宽松）----
    let mut header_only = ELISE_2.to_vec();
    assert_eq!(
        (header_only[10], header_only[11]),
        (0x00, 0x03),
        "夹具的 MThd ntracks 实测是 3（偏移 10..12，大端 u16）"
    );
    header_only[11] = 0x02;
    assert_eq!(
        read(&header_only),
        intact,
        "只改 MThd ntracks 不改变任何读数：两个解析器都读到文件末尾"
    );

    // ---- 注入 2：截掉最后一条 MTrk chunk ⇒ 真的丢轨道 ----
    let chunks = track_chunks(ELISE_2).expect("chunk 布局");
    assert_eq!(chunks.len(), 4);
    let last_mtrk_start = chunks[3].payload.start - 8;
    assert_eq!(
        last_mtrk_start, 3836,
        "最后一条 MTrk 的 chunk 头在偏移 3836"
    );
    let mut dropped = ELISE_2[..last_mtrk_start].to_vec();
    dropped[11] = 0x02; // 让 ntracks 自洽

    let broken = read(&dropped);
    assert_eq!(broken.mtrk, 2, "只剩 2 条 MTrk");
    assert_eq!(broken.notes, 517, "只剩通道 0 的 517 颗音符");
    assert_eq!(broken.per_channel, BTreeMap::from([(0u8, 517usize)]));
    assert_eq!(broken.max_end_tick, Some(75_120), "通道 0 仍跨到 75120");
    assert!(
        !channel_one_diff_holds(&one, &broken),
        "丢掉一条轨道后，判据 ③的谓词必须为 false（否则判据 ③没有牙齿）"
    );

    // ---- 还原：谓词回到 true，且字节与夹具逐字节相同 ----
    let restored = read(ELISE_2);
    assert!(channel_one_diff_holds(&one, &restored), "还原后谓词为真");
    assert_eq!(restored, intact, "还原后读数逐字段相同");
    assert_eq!(ELISE_2.len(), 6687, "夹具本身没有被改动");
}

/// 判据 ⑦（**登记缺陷，不是承诺**）: 真文件的 tempo map 往返**不保真**。
///
/// 实测的两步：
/// 1. `parse_smf` 为"孤儿拍号"（同一 tick 上没有前驱 tempo 的拍号）合成一条
///    `500000 µs` 的 tempo（`src/midi.rs:665-671`）。
/// 2. `to_smf_bytes` 把 tempo 事件的排序键设成 `rank 2`、拍号设成 `rank 3`
///    （`src/midi.rs:272-281`）⇒ 同一 tick 上**全部** tempo 排在**全部**拍号之前。
///    回读时 `parse_smf` 把拍号挂到"同一 tick 的**最后一条** tempo"上
///    （`src/midi.rs:661`）⇒ 配对换人。
///
/// 这条判据把当前的字面读数钉住。日后有人修它，这条会**故意**变红。
/// 修它需要裁决 `MidiTempo`（把 tempo 与拍号合成一个结构）的语义，
/// 那会改变 `yeban-mcp` 的 `yeban_export_midi` 输出字节 ⇒ **不在本票范围**。
#[test]
fn measured_tempo_map_round_trip_is_lossy_pending_adjudication() {
    let first = parse_smf(ELISE_2).expect("接受");
    // 原始：3/8 挂在 500000 上。
    assert_eq!(
        first.tempos,
        vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: 500_000,
                numerator: Some(3),
                denominator_pow2: Some(3),
            },
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: 833_333,
                numerator: Some(4),
                denominator_pow2: Some(2),
            },
        ],
        "原始真文件的 tempo map 实测读数"
    );

    let reemitted = reexport(&first, MidiFormat::Parallel)
        .to_smf_bytes()
        .expect("编码");
    let second = parse_smf(&reemitted).expect("回读");

    // 音符仍然保真 ⇒ 缺陷被隔离在 tempo map 一层。
    assert_eq!(
        sorted_keys(&second),
        sorted_keys(&first),
        "音符往返仍然逐键相同"
    );
    assert_eq!(second.ppq, first.ppq);

    // 字面读数：3/8 的配对丢失，500000 那条变成"无拍号"。
    assert_eq!(
        second.tempos,
        vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: 500_000,
                numerator: None,
                denominator_pow2: None,
            },
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: 833_333,
                numerator: Some(4),
                denominator_pow2: Some(2),
            },
        ],
        "登记：同一 tick 上的 tempo/拍号配对在往返后换人"
    );
    assert_ne!(
        second.tempos, first.tempos,
        "这条判据的存在理由：往返**不**保真；修好后请改成 assert_eq!"
    );
    // 条目数不变，因此"丢配对"不是"丢事件"。
    assert_eq!(
        second.tempos.len(),
        first.tempos.len(),
        "tempo 条目数不变，只有配对变了"
    );

    // 对照：tick 互不相同的 tempo map 往返是保真的
    //（既有单元测试 `midi::tests::tempo_map_round_trips` 覆盖这一情形）。
    let distinct = MidiExport {
        format: MidiFormat::Parallel,
        ppq: 480,
        tempos: vec![
            MidiTempo::with_time_signature(120.0, 4, 2),
            MidiTempo {
                tick: 960,
                ..MidiTempo::from_bpm(90.0)
            },
        ],
        tracks: vec![MidiExportTrack {
            name: "t".into(),
            channel: 0,
            notes: vec![MidiNote::new(EntityId::new(), 0, 60, 120)],
        }],
    };
    let round = parse_smf(&distinct.to_smf_bytes().expect("编码")).expect("回读");
    assert_eq!(
        round.tempos, distinct.tempos,
        "tick 互不相同时 tempo map 往返保真 ⇒ 缺陷只在同一 tick 的多条 tempo 上"
    );
}

/// 判据 ⑧: 从外面看，真文件的拒绝路径保持沉默（不存在假阳性拒绝）。
///
/// 这是"接受"判据的守卫：只要哪天解析器开始拒绝真文件，这里先红。
#[test]
fn the_real_files_do_not_trip_any_documented_rejection() {
    for (label, bytes) in [
        ("WoO 59", WOO59),
        ("Elise 1", ELISE_1),
        ("Elise 2", ELISE_2),
    ] {
        match parse_smf(bytes) {
            Ok(_) => {}
            Err(MidiError::Decode(detail)) => {
                panic!("{label}: 真文件不得触发 Decode：{detail}");
            }
            Err(other) => panic!("{label}: 真文件不得触发任何拒绝，实际 {other:?}"),
        }
        // chunk 头声明长度必须与文件长度自洽：最后一个 chunk 的负载必须收到末尾。
        let chunks = track_chunks(bytes).expect("chunk 布局合法");
        assert_eq!(
            chunks.last().map(|c| c.payload.end),
            Some(bytes.len()),
            "{label}: 最后一个 chunk 必须收到文件末尾"
        );
        assert_eq!(
            chunks.first().map(|c| c.fourcc),
            Some(*b"MThd"),
            "{label}: 第一个 chunk 必须是 MThd"
        );
        assert_eq!(
            chunks.first().map(|c| c.len()),
            Some(6),
            "{label}: MThd 负载是 6 字节"
        );
    }
}
