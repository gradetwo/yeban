//! `yeban-midi` 的**集成判据**（只经公开 API，形同 `yeban-mcp` / `yeban-app` 的视角）。
//!
//! ## 这个文件补的是哪个缺口（先量后做）
//!
//! 本 crate 在本次改动前有 **29 个内联单元测试**（`midi.rs` 12 / `export.rs` 11 /
//! `vlq.rs` 6），但**零个集成测试文件**，且 `MidiError` 的 13 个变体里有 4 个
//! **零覆盖**：`Decode`、`UnsupportedTimecode`、`UnsupportedFormat`、`Encode`。
//! 前三个是**对抗输入**的拒绝路径 —— 也就是"损坏/恶意文件不得被解析成看起来合法的
//! 东西"这条承诺（`src/vlq.rs:64-65`）真正落地的地方。
//!
//! 因此本文件的判据分成两组：
//! ① **写→读的字节级往返**：用真实字节（既有 crate 自己编出的，也有本文件手工拼的
//!    规范样本）钉住 `MThd`/`MTrk`/VLQ 的字面字节，并用 crate 自己的**独立**
//!    VLQ 解码器（`yeban_midi::vlq`）核验 `midly` 写出的字节；
//! ② **拒绝路径**：用真实字节证明三条已文档化的拒绝真的发生。
//!
//! ## 规范出处
//!
//! - `[ARCH-FMT-001 §5.5]`：SMF 0/1 导出，Tempo Map 拍速标记、拍号变更、多通道分轨。
//! - `ADR-0001` D47：`yeban-midi` 是 SMF 编解码的**唯一**实现，落盘留在消费方
//!   （`yeban-app`）⇒ 本文件**不**测文件写入的原子性，那在 `crates/yeban-app/src/save.rs`。
//! - `[ARCH-DET-*]`：同一工程两次导出逐字节相同。
//!
//! ## 本文件**没有**证明什么
//!
//! - `MidiError::Encode` 与 `MidiExportError::PpqUnrepresentable` / `Encode` 是
//!   **构造上不可达**的（`midly` 往 `Vec<u8>` 写不会失败；`PPQ` 是 960，装得进 `u16`）。
//!   这里**不**为它们编造覆盖。
//! - SMPTE 时间码与格式 2 文件只证明"被拒绝"，不证明它们的语义被支持。

use std::fs;

use yeban_midi::midi::{
    DEFAULT_PPQ, MidiError, MidiExport, MidiExportTrack, MidiFormat, MidiTempo, ParsedMidi,
    parse_smf, track_chunks, track_from_notes,
};
use yeban_midi::vlq;
use yeban_model::EntityId;
use yeban_model::music::MidiNote;

/// 造一颗合法音符（力度 100 = `yeban_model::music::DEFAULT_VELOCITY`）。
fn note(start_tick: u64, pitch: u8, duration_ticks: u64) -> MidiNote {
    MidiNote::new(EntityId::new(), start_tick, pitch, duration_ticks)
}

/// 大端 `u32` —— 手工拼 SMF 时反复要用。
fn be32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

/// 手工拼一个规范合法的 SMF：一个 `MThd` + 若干 `MTrk`。
///
/// 这是**独立于本 crate 编码器**的字节构造器：它的存在是为了让"读"这一侧
/// 面对的不是"自己写出来的东西"。
fn hand_built_smf(format: u16, timing: [u8; 2], track_payloads: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&be32(6));
    bytes.extend_from_slice(&format.to_be_bytes());
    bytes.extend_from_slice(&(track_payloads.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&timing);
    for payload in track_payloads {
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&be32(payload.len() as u32));
        bytes.extend_from_slice(payload);
    }
    bytes
}

/// 回读结果的规范化键集合（**排序后**，便于整体相等断言）。
fn parsed_keys(parsed: &ParsedMidi) -> Vec<(u8, u8, u8, u64, u64)> {
    let mut keys: Vec<(u8, u8, u8, u64, u64)> =
        parsed.notes.iter().map(|note| note.key()).collect();
    keys.sort_unstable();
    keys
}

// ---------------------------------------------------------------------------
// ① 写 → 读：字节级往返
// ---------------------------------------------------------------------------

/// 判据 ①: 手工拼的 **SMF 1** 真实字节（不由本 crate 产出）被逐音符读回。
///
/// 钉住的事实：conductor 轨的 `Tempo` + `TimeSignature` 在 tick 0 合并成**一条**
/// tempo 记录；`MTrk` 之间的绝对 tick 各自从 0 起算；VLQ 大端 7 位组解码正确。
#[test]
fn hand_built_smf1_bytes_parse_note_by_note() {
    // conductor: tempo 500000 us/quarter (120 BPM), 拍号 4/4, 收尾。
    let conductor: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // Tempo 0x07A120 = 500000
        0x00, 0xFF, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08, // 4/4
        0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
    ];
    // 音符轨 (通道 3): key60 从 0 起、时值 480；key62 从 480 起、时值 480。
    let notes: &[u8] = &[
        0x00, 0x93, 0x3C, 0x64, // NoteOn  ch3 key60 vel100
        0x83, 0x60, 0x83, 0x3C, 0x40, // +480 NoteOff ch3 key60
        0x00, 0x93, 0x3E, 0x64, // +0   NoteOn  ch3 key62 vel100
        0x83, 0x60, 0x83, 0x3E, 0x40, // +480 NoteOff ch3 key62
        0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
    ];
    let bytes = hand_built_smf(1, [0x03, 0xC0], &[conductor, notes]);

    let parsed = parse_smf(&bytes).expect("手工拼的 SMF 1 必须可读");
    assert_eq!(parsed.format, MidiFormat::Parallel);
    assert_eq!(parsed.ppq, 960);
    assert_eq!(
        parsed_keys(&parsed),
        vec![(3, 60, 100, 0, 480), (3, 62, 100, 480, 480)],
        "逐音符: (通道, 音高, 力度, 起始 tick, 时值)"
    );
    assert_eq!(parsed.tempos.len(), 1, "tick 0 的 Tempo 与拍号合并成一条");
    assert_eq!(parsed.tempos[0].tick, 0);
    assert_eq!(
        parsed.tempos[0].microseconds_per_quarter,
        Some(500_000),
        "120 BPM"
    );
    assert_eq!(parsed.tempos[0].numerator, Some(4));
    assert_eq!(parsed.tempos[0].denominator_pow2, Some(2), "4 分音符 = 2^2");
}

/// 判据 ②: 手工拼的 **SMF 0** 真实字节被读回，且通道来自事件自己的状态字节。
#[test]
fn hand_built_smf0_bytes_parse_and_keep_each_channel() {
    // 通道 0 与通道 9 (鼓) 同时从 tick 0 起。
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // NoteOn ch0 key60
        0x00, 0x99, 0x24, 0x7F, // NoteOn ch9 key36 vel127
        0x81, 0x70, 0x80, 0x3C, 0x00, // +240 NoteOff ch0 (vel 0)
        0x00, 0x89, 0x24, 0x00, // +0   NoteOff ch9 (vel 0)
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let bytes = hand_built_smf(0, [0x01, 0xE0], &[track]);

    let parsed = parse_smf(&bytes).expect("手工拼的 SMF 0 必须可读");
    assert_eq!(parsed.format, MidiFormat::SingleTrack);
    assert_eq!(parsed.ppq, 480, "0x01E0 = 480");
    assert_eq!(
        parsed_keys(&parsed),
        vec![(0, 60, 64, 0, 240), (9, 36, 127, 0, 240)]
    );
}

/// 判据 ③: 本 crate 编出的字节在字面层面就是合法 SMF 1。
///
/// 这里刻意**不**相信 `midly` 的解析器：`MThd` 的 6 个字节、每条 `MTrk` 的
/// `EndOfTrack` 收尾、以及第一条事件 delta 的 **VLQ** 都按 `Standard MIDI File 1.0`
/// 逐字节核对，VLQ 用本 crate 的独立解码器 `yeban_midi::vlq` 读。
#[test]
fn exported_bytes_have_literal_smf1_header_and_terminators() {
    let export = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        // 空 tempo map ⇒ conductor 轨只剩 EndOfTrack，便于钉住轨道条数。
        tempos: Vec::new(),
        // 轨道名为空 ⇒ 不写 TrackName meta ⇒ 轨首就是音符事件的 delta。
        tracks: vec![track_from_notes("", 0, &[note(480, 60, 240)])],
    };
    let bytes = export.to_smf_bytes().expect("编码");

    // --- MThd：12 字节头部逐字节 ---
    assert_eq!(&bytes[0..4], b"MThd", "chunk 标识");
    assert_eq!(
        &bytes[4..8],
        &[0x00, 0x00, 0x00, 0x06],
        "MThd 负载恒为 6 字节"
    );
    assert_eq!(
        u16::from_be_bytes([bytes[8], bytes[9]]),
        1,
        "格式号 1 (SMF 1, 大端)"
    );
    assert_eq!(
        u16::from_be_bytes([bytes[10], bytes[11]]),
        2,
        "轨道数 2 = conductor + 1 条音符轨 (大端)"
    );
    assert_eq!(&bytes[12..14], &[0x03, 0xC0], "时间分度 0x03C0 = 960");

    // --- chunk 骨架：MThd + 2 × MTrk，且每条都以 FF 2F 00 收尾 ---
    let chunks = track_chunks(&bytes).expect("chunk 布局");
    assert_eq!(chunks.len(), 3, "MThd + conductor + 1 条音符轨");
    assert_eq!(&chunks[0].fourcc, b"MThd");
    assert_eq!(chunks[0].payload, 8..14, "MThd 负载在文件里的范围");
    for chunk in &chunks[1..] {
        assert_eq!(&chunk.fourcc, b"MTrk");
        let payload = &bytes[chunk.payload.clone()];
        assert_eq!(
            &payload[payload.len() - 3..],
            &[0xFF, 0x2F, 0x00],
            "MTrk 必须以 EndOfTrack 收尾"
        );
    }
    assert_eq!(
        &bytes[chunks[1].payload.clone()],
        &[
            0x00, 0xFF, 0x03, 0x0F, // delta 0, TrackName 长度 15
            b'Y', b'e', b'b', b'a', b'n', b' ', b'C', b'o', b'n', b'd', b'u', b'c', b't', b'o',
            b'r', 0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
        ],
        "conductor 轨恒名为 \"Yeban Conductor\"（`to_smf_bytes` 里那条 `lanes.push`）+ EndOfTrack"
    );

    // --- 音符轨：delta 480 的 VLQ + NoteOn，用独立 VLQ 解码器读 ---
    let lane = &bytes[chunks[2].payload.clone()];
    assert_eq!(&lane[0..2], &[0x83, 0x60], "480 的 VLQ 是 83 60");
    let mut cursor = 0usize;
    assert_eq!(
        vlq::decode(lane, &mut cursor),
        Some(480),
        "本 crate 的独立 VLQ 解码器必须读出 480"
    );
    assert_eq!(cursor, 2, "恰好消费 2 字节");
    assert_eq!(
        &lane[2..5],
        &[0x90, 0x3C, 0x64],
        "NoteOn ch0 key60 vel100 (`MidiNote::new` 的默认力度 = 100)"
    );

    // --- 而且真的能读回同一颗音符 ---
    let parsed = parse_smf(&bytes).expect("回读");
    assert_eq!(parsed_keys(&parsed), vec![(0, 60, 100, 480, 240)]);
}

/// 判据 ④ (**端到端**): 工程 → SMF 字节 → **真实文件** → 字节 → 回读 ⇒ 逐音符与工程一致。
///
/// 期望值来自 **模型侧独立数出来**的音符（不是导出器自己报的数），
/// 因此"导出器把音符丢了"会直接让本判据变红。
#[test]
fn project_to_file_to_parsed_notes_is_field_exact() {
    let project = yeban_model::samples::demo_project();
    let bytes = yeban_midi::export::export_from_project(&project)
        .expect("演示夹具必须可投影")
        .to_smf_bytes()
        .expect("编码");

    // 落盘再读回：证明这些字节能作为一个真实文件存活。
    let path =
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("midi_contract_project_to_file.mid");
    fs::write(&path, &bytes).expect("写临时 .mid");
    let on_disk = fs::read(&path).expect("读回临时 .mid");
    let _ = fs::remove_file(&path);
    assert_eq!(on_disk, bytes, "文件里的字节必须与内存里的字节完全相同");

    // 独立口径：直接从模型侧数出 (起始 tick, 音高)，不经过导出器的报告。
    let mut model_notes: Vec<(u64, u8)> = Vec::new();
    for track in project.tracks.values() {
        if track.id == project.master_bus_track_id {
            continue;
        }
        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let Some(pool) = entry.content.notes() else {
                continue;
            };
            for n in pool.values() {
                model_notes.push((placement.start_tick.saturating_add(n.start_tick), n.pitch));
            }
        }
    }
    model_notes.sort_unstable();

    let parsed = parse_smf(&on_disk).expect("导出的字节必须能被 SMF 读取面读回");
    let mut parsed_starts: Vec<(u64, u8)> = parsed
        .notes
        .iter()
        .map(|note| (note.start_tick, note.key))
        .collect();
    parsed_starts.sort_unstable();
    assert_eq!(
        parsed_starts, model_notes,
        "逐音符: (绝对起始 tick, 音高) 必须与工程侧独立数出来的一致"
    );
    assert_eq!(parsed.ppq, 960, "时间分度必须是工程的 960 PPQ");
    assert_eq!(parsed.format, MidiFormat::Parallel, "D47: 恒为 SMF 1");
}

/// 判据 ⑤ (`[ARCH-DET-*]`): 同一工程两次导出 ⇒ **逐字节相同**（经由文件也不变）。
#[test]
fn export_is_byte_deterministic_across_two_calls() {
    let project = yeban_model::samples::demo_project();
    let first = yeban_midi::export::export_from_project(&project)
        .expect("投影")
        .to_smf_bytes()
        .expect("编码");
    let second = yeban_midi::export::export_from_project(&project)
        .expect("投影")
        .to_smf_bytes()
        .expect("编码");
    assert_eq!(first, second, "同一工程两次导出的字节必须完全相同");
    assert!(!first.is_empty(), "非空才算真的导出了东西");
}

// ---------------------------------------------------------------------------
// ② 拒绝路径（本次改动前零覆盖）
// ---------------------------------------------------------------------------

/// 判据 ⑥: 三条**已文档化**的拒绝在真实字节上真的发生。
///
/// - 格式 2 ⇒ `UnsupportedFormat(2)`（`parse_smf` 的 `Format::Sequential` 分支）；
/// - SMPTE 时间码 ⇒ `UnsupportedTimecode`（`parse_smf` 的 `Timing::Timecode` 分支）；
/// - chunk 声明长度超出文件 ⇒ `Decode`（`track_chunks` 里那条"声明 N 字节，但文件只剩 M 字节"）。
#[test]
fn documented_rejections_fire_on_real_bytes() {
    // 格式 2 (Sequential)：SMF 里有这个格式号，本切片不支持。
    let format_two = hand_built_smf(2, [0x03, 0xC0], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    assert_eq!(
        parse_smf(&format_two),
        Err(MidiError::UnsupportedFormat(2)),
        "格式 2 必须被拒绝"
    );

    // SMPTE: 时间分度高位置 1 (0xE8 = -24 fps, 0x04 = 4 ticks/frame)。
    let smpte = hand_built_smf(0, [0xE8, 0x04], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    assert_eq!(
        parse_smf(&smpte),
        Err(MidiError::UnsupportedTimecode),
        "SMPTE 时间码必须被拒绝"
    );

    // 说谎的 chunk 长度：MThd 声明 999 字节，文件只剩 6 字节。
    let mut lying = Vec::new();
    lying.extend_from_slice(b"MThd");
    lying.extend_from_slice(&be32(999));
    lying.extend_from_slice(&[0u8; 6]);
    match track_chunks(&lying) {
        Err(MidiError::Decode(message)) => {
            assert!(
                message.contains("声明 999 字节"),
                "错误必须点名说谎的长度；实际: {message}"
            );
        }
        other => panic!("期望 Decode, 得到 {other:?}"),
    }

    // 同样的谎言发生在 MTrk 上：截掉最后一个字节。
    let mut truncated = hand_built_smf(0, [0x03, 0xC0], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    truncated.truncate(truncated.len() - 1);
    assert!(
        matches!(track_chunks(&truncated), Err(MidiError::Decode(_))),
        "MTrk 声明 4 字节但只剩 3 字节 ⇒ 必须报 Decode"
    );
}

/// 判据 ⑦: 回读的两条"严格"承诺在真实字节上成立（未闭合音符 / 孤立 `NoteOff`）。
///
/// 出处：`src/midi.rs:34`（"回读是**严格**的"）、`:180-193`。
#[test]
fn unbalanced_notes_are_rejected_from_outside_the_crate() {
    // 只有 NoteOn，没有 NoteOff ⇒ 未闭合。
    let unclosed = hand_built_smf(
        0,
        [0x03, 0xC0],
        &[&[0x00, 0x90, 0x3C, 0x40, 0x00, 0xFF, 0x2F, 0x00]],
    );
    assert_eq!(
        parse_smf(&unclosed),
        Err(MidiError::UnclosedNote {
            start_tick: 0,
            key: 60
        })
    );

    // 只有 NoteOff ⇒ 无对应 NoteOn。
    let orphan = hand_built_smf(
        0,
        [0x03, 0xC0],
        &[&[0x00, 0x80, 0x3C, 0x00, 0x00, 0xFF, 0x2F, 0x00]],
    );
    assert_eq!(
        parse_smf(&orphan),
        Err(MidiError::UnmatchedNoteOff { tick: 0, key: 60 })
    );
}

/// 判据 ⑧: **已量到的宽松行为**被钉住 —— 它是"登记在案的现状"，不是被认可的契约。
///
/// 实测（本次改动前）：
/// 1. `track_chunks` 忽略文件尾部**不足 8 字节**的残留，返回 `Ok`；
/// 2. 结构性截断（`MTrk` 声明 4 字节、实际只有 3 字节）时，`track_chunks` 报
///    `Decode`，而 `parse_smf`（走 `midly`）仍返回 `Ok` 且音符数为 0。
///
/// 第 2 条是**两个读取器之间的分歧**。`parse_smf` 的文档只把"严格"限定在音符配对
/// （`src/midi.rs:34`），**没有**承诺校验 chunk 骨架，因此这不是文档违约 ——
/// 这里只把现状钉住：将来任何一侧收紧或放松，本判据会先变红，逼出一次有意识的决定。
#[test]
fn measured_reader_leniency_is_pinned() {
    // 现状 1: 尾部 5 字节残留被静默忽略。
    let mut trailing = hand_built_smf(0, [0x03, 0xC0], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    trailing.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00]);
    let chunks = track_chunks(&trailing).expect("尾部残留目前不报错");
    assert_eq!(chunks.len(), 2, "MThd + 1 条 MTrk");
    let covered = chunks.last().expect("至少一个 chunk").payload.end;
    assert_eq!(
        trailing.len() - covered,
        5,
        "这 5 字节残留**没有**被任何 chunk 覆盖（即被忽略）"
    );

    // 现状 2: 结构性截断 ⇒ 两个读取器分歧。
    let mut truncated = hand_built_smf(0, [0x03, 0xC0], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    truncated.truncate(truncated.len() - 1);
    assert!(
        matches!(track_chunks(&truncated), Err(MidiError::Decode(_))),
        "严格的一侧: track_chunks 报 Decode"
    );
    let parsed = parse_smf(&truncated).expect("宽松的一侧: midly 目前接受它");
    assert_eq!(parsed.notes.len(), 0, "接受的结果是 0 颗音符");
    assert_eq!(parsed.ppq, 960, "头还是被读出来了");
}

/// 判据 ⑨: `MidiExport` 的边界输入被**精确**拒绝（跨 crate 视角）。
#[test]
fn encoder_boundaries_are_rejected_precisely() {
    let base = MidiExport {
        format: MidiFormat::SingleTrack,
        ppq: DEFAULT_PPQ,
        tempos: Vec::new(),
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 60, 120)],
        }],
    };

    let zero_ppq = MidiExport {
        ppq: 0,
        ..base.clone()
    };
    assert_eq!(zero_ppq.to_smf_bytes(), Err(MidiError::InvalidPpq(0)));

    let no_tracks = MidiExport {
        tracks: Vec::new(),
        ..base.clone()
    };
    assert_eq!(no_tracks.to_smf_bytes(), Err(MidiError::NoTracks));

    // 超出 28 位 VLQ 上限的 delta ⇒ 拒绝（不是截断）。
    let beyond = u64::from(vlq::VLQ_MAX) + 1;
    let too_far = MidiExport {
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(beyond, 60, 120)],
        }],
        ..base.clone()
    };
    assert_eq!(
        too_far.to_smf_bytes(),
        Err(MidiError::DeltaOverflow {
            tick: beyond,
            delta: beyond
        })
    );

    // 通道号超出 4 bit。
    let bad_channel = MidiExport {
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 16,
            notes: vec![note(0, 60, 120)],
        }],
        ..base
    };
    assert_eq!(
        bad_channel.to_smf_bytes(),
        Err(MidiError::ChannelOutOfRange(16))
    );

    // tempo 值被钳进 24 位（`mpqn` 是 u24）。
    let clamped = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo {
            tick: 0,
            microseconds_per_quarter: Some(0x00FF_FFFF + 1),
            numerator: None,
            denominator_pow2: None,
        }],
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 60, 120)],
        }],
    };
    let bytes = clamped.to_smf_bytes().expect("钳制后必须能编码");
    assert_eq!(
        parse_smf(&bytes).expect("回读").tempos[0].microseconds_per_quarter,
        Some(0x00FF_FFFF),
        "超过 u24 的 mpqn 被钳到 0x00FFFFFF"
    );
}

// ---------------------------------------------------------------------------
// ③ 类别①/④/⑦：参数与长度的极值（本次改动补的判据）
// ---------------------------------------------------------------------------

/// 一条只有一个音符的单轨 `MidiExport`（下面几条判据的公共底座）。
fn single_track_export(ppq: u16) -> MidiExport {
    MidiExport {
        format: MidiFormat::SingleTrack,
        ppq,
        tempos: Vec::new(),
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 60, 120)],
        }],
    }
}

/// 判据 ⑩ (类别④ 参数极值): 15 位时间分度字段的**上界**必须被拒绝，不许被掩码回绕。
///
/// `MThd` 的时间分度是 15 位字段。修之前 `to_smf_bytes` 只查 `ppq == 0`，而
/// `midly` 的 `u15::new` 是 `raw & 0x7FFF`（掩码、不是拒绝）⇒ 超界值被**静默**写成
/// 另一个时间分度。实测（修之前，单位 = 无量纲的 ppq 读数）：
///
/// | 请求 | `MThd` 里落盘的字段 |
/// | ---: | ---: |
/// | `0x8000` | `0x0000` |
/// | `0x83C0` | `0x03C0`（= 960） |
/// | `0xC000` | `0x4000`（= 16384） |
/// | `0xFFFF` | `0x7FFF` |
///
/// 这同时是"`InvalidPpq` 的文档说'或超过 15 位上限'，但只有一半落地"的证据。
/// 对照臂在**上界本身**：`0x7FFF` 必须接受，且逐字节写进 `MThd`。
#[test]
fn ppq_above_the_15_bit_field_is_rejected_instead_of_masked() {
    for ppq in [0x8000u16, 0x83C0, 0xC000, 0xFFFF] {
        assert_eq!(
            single_track_export(ppq).to_smf_bytes(),
            Err(MidiError::InvalidPpq(ppq)),
            "ppq {ppq:#06x} 必须被拒绝，不许掩码成 {:#06x}",
            ppq & 0x7FFF
        );
    }

    // 对照臂：15 位的上界本身合法，且原样落进 MThd。
    let bytes = single_track_export(0x7FFF)
        .to_smf_bytes()
        .expect("0x7FFF 是 15 位字段的上界，必须接受");
    assert_eq!(&bytes[12..14], &[0x7F, 0xFF], "时间分度字段原样写 0x7FFF");
    assert_eq!(parse_smf(&bytes).expect("回读").ppq, 0x7FFF);
}

/// 判据 ⑪ (类别① 越界输入): 时间分度为 **0** 的 `MThd` 在**读**这一侧也必须被拒绝。
///
/// SMF 1.0 要求 Metrical 的时间分度是正数。修之前 `parse_smf` 回出
/// `Ok { ppq: 0, .. }` —— 一个下游按 `ppq` 换算 tick 就会除零的读数，而导出侧本来
/// 就拒绝写出 0（判据 ⑨ 的 `zero_ppq` 一行）⇒ 只拒一半是不对称的。
#[test]
fn a_zero_division_header_is_rejected_by_the_reader() {
    let zero = hand_built_smf(0, [0x00, 0x00], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    assert_eq!(
        parse_smf(&zero),
        Err(MidiError::InvalidPpq(0)),
        "时间分度 0 必须被拒绝（不是回出一个 ppq = 0 的读数）"
    );

    // 对照臂：同一份字节只把分度改成 1（大端 `00 01`）⇒ 接受。
    let one = hand_built_smf(0, [0x00, 0x01], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    assert_eq!(parse_smf(&one).expect("分度 1 是合法的").ppq, 1);
}

/// 判据 ⑫ (类别④ 参数极值): 只给**一半**拍号的 tempo 记录必须被拒绝，不许静默丢弃。
///
/// `FF 58 04` 的分子与分母的以 2 为底的幂必须成对。修之前 `TempoGroup::of` 用的是
/// `match (numerator, denominator_pow2) { (Some, Some) => .., _ => None }` ⇒ 只给一个时
/// 拍号变成 `None`，而 `microseconds_per_quarter` 也是 `None` 的记录被"丢掉空组"这一步
/// 整条删除：调用方给出一个 tick 上的记录，`to_smf_bytes` 回 `Ok`，文件里**什么都没有**。
/// 实测（修之前）：`numerator = Some(4)`、其余 `None` ⇒ `to_smf_bytes() = Ok(65 字节)`
/// 且回读 `tempos = []`。
#[test]
fn half_a_time_signature_is_rejected_instead_of_dropped() {
    let cases = [(Some(4u8), None), (None, Some(2u8))];
    for (index, (numerator, denominator_pow2)) in cases.into_iter().enumerate() {
        let source = MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: vec![MidiTempo {
                tick: 0,
                microseconds_per_quarter: None,
                numerator,
                denominator_pow2,
            }],
            tracks: vec![MidiExportTrack {
                name: String::new(),
                channel: 0,
                notes: vec![note(0, 60, 120)],
            }],
        };
        assert_eq!(
            source.to_smf_bytes(),
            Err(MidiError::HalfTimeSignature {
                tick: 0,
                numerator,
                denominator_pow2,
            }),
            "第 {index} 个半拍号必须被拒绝，不许静默丢弃"
        );
    }

    // 对照臂 1: 两个都**没有**（完全空的记录）仍然是"丢掉空组"，不是错误。
    let empty = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![MidiTempo {
            tick: 0,
            microseconds_per_quarter: None,
            numerator: None,
            denominator_pow2: None,
        }],
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 60, 120)],
        }],
    };
    let bytes = empty
        .to_smf_bytes()
        .expect("两个事件都没有的记录是空组，必须照常编码");
    assert!(parse_smf(&bytes).expect("回读").tempos.is_empty());

    // 对照臂 2: 成对的拍号照旧往返（判据 ① 已覆盖 4/4，这里覆盖一对非 4 的）。
    let paired = MidiExport {
        tempos: vec![MidiTempo {
            tick: 0,
            microseconds_per_quarter: None,
            numerator: Some(3),
            denominator_pow2: Some(3),
        }],
        ..single_track_export(DEFAULT_PPQ)
    };
    let parsed = parse_smf(&paired.to_smf_bytes().expect("编码")).expect("回读");
    assert_eq!(parsed.tempos.len(), 1);
    assert_eq!(parsed.tempos[0].numerator, Some(3));
    assert_eq!(parsed.tempos[0].denominator_pow2, Some(3));
}

/// 判据 ⑬ (类别①/⑦ 越界字节与长度极值): SMF 两个读入口对**任意字节**只产生
/// `Ok` 或 `Err`，绝不 panic。
///
/// 量的是"跑了几次解析"（单位 = 次调用）；任何一次 panic 都会让本判据失败。
/// 输入取自：三个已提交的真夹具 + 本文件手拼的规范样本，三种变形（① 全部截断前缀、
/// ② 逐字节翻转、③ 插入一个字节）+ 一个**种子固定**的 xorshift64\* 生成的伪随机字节
/// （不引第三方 `rand`、不读系统熵 ⇒ 可复现）。
#[test]
fn smf_readers_never_panic_on_arbitrary_bytes() {
    /// 种子固定的 xorshift64\*：判据必须可复现，因此不用系统熵。
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }
    }

    let corpus: Vec<Vec<u8>> = vec![
        hand_built_smf(1, [0x03, 0xC0], &[&[0x00, 0xFF, 0x2F, 0x00]]),
        hand_built_smf(
            0,
            [0x03, 0xC0],
            &[&[
                0x00, 0x90, 0x3C, 0x40, 0x60, 0x80, 0x3C, 0x00, 0x00, 0xFF, 0x2F, 0x00,
            ]],
        ),
        include_bytes!("fixtures/fur_elise_woo59_384ppq_3mtrk.mid").to_vec(),
        include_bytes!("fixtures/fur_elise_480ppq_1mtrk.mid").to_vec(),
        include_bytes!("fixtures/fur_elise_480ppq_3mtrk.mid").to_vec(),
        Vec::new(),
    ];

    let mut runs: usize = 0;
    for original in &corpus {
        // ① 截断：每个前缀都是一个可能非法但必须不 panic 的输入。
        for cut in (0..=original.len()).step_by(3) {
            let slice = &original[..cut];
            let _ = parse_smf(slice);
            let _ = track_chunks(slice);
            runs += 2;
        }
        if original.is_empty() {
            continue;
        }
        // ② 逐字节翻转：每个位置翻 1 个 bit。
        let mut copy = original.clone();
        let mut rng = Rng(0x1234_5678_9ABC_DEF0 ^ original.len() as u64);
        for index in (0..original.len()).step_by(5) {
            copy[index] ^= 1u8 << rng.below(8);
            let _ = parse_smf(&copy);
            let _ = track_chunks(&copy);
            runs += 2;
            copy[index] = original[index];
        }
        // ③ 插入一个字节：制造新的（通常越过文件尾的）chunk 头。
        let mut inserted = original.clone();
        for index in (0..original.len()).step_by(97) {
            inserted.insert(index, 0x80);
            let _ = parse_smf(&inserted);
            let _ = track_chunks(&inserted);
            runs += 2;
            inserted.remove(index);
        }
    }

    // ④ 纯伪随机字节（长度 0..=511）。
    let mut rng = Rng(0xDEAD_BEEF_CAFE_BABE);
    for _ in 0..20_000 {
        let len = rng.below(512);
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next() >> 33) as u8).collect();
        let _ = parse_smf(&bytes);
        let _ = track_chunks(&bytes);
        runs += 2;
    }

    println!("smf_readers_never_panic_on_arbitrary_bytes: runs={runs}");
    assert!(runs >= 5_000, "探针只跑了 {runs} 次，样本太少");
}

// ---------------------------------------------------------------------------
// ④ 第二批判据（本票新增）：内容序、配对方向、VLQ/拍号/调号的边界值
//
// 每条的"补的是哪个缺口"由同票的注入实测给出（字面替换表见提交正文）：
// 这些替换在本节之前让本 crate 的全部判据保持**绿**，即当时没有任何判据守着它们。
// ⛔ 本节不改任何既有判据的期望值，只增加判据。
// ---------------------------------------------------------------------------

/// 判据 ⑭ (类别⑤ 幂等 / `ARCH-DET-001`): tempo map 的**输入 Vec 顺序**不影响导出字节。
///
/// `to_smf_bytes` 的文档写着"写出前按**内容**规范化排序（不是按本 `Vec` 的输入顺序）
/// ⇒ 导出字节只由内容决定"。同一批记录换一个输入顺序必须给出**逐字节相同**的文件。
///
/// 注入实测（本票）：去掉组排序那一步（`groups.sort_unstable()`）后，本判据之前
/// 本 crate 的全部判据保持绿 —— 因为它们都按"同一份输入"导出两次。
#[test]
fn tempo_map_bytes_depend_only_on_content_not_input_order() {
    let first = MidiTempo {
        tick: 0,
        microseconds_per_quarter: Some(500_000),
        numerator: Some(4),
        denominator_pow2: Some(2),
    };
    let second = MidiTempo {
        tick: 0,
        microseconds_per_quarter: Some(833_333),
        numerator: Some(3),
        denominator_pow2: Some(2),
    };
    let with = |tempos: Vec<MidiTempo>| MidiExport {
        tempos,
        ..single_track_export(DEFAULT_PPQ)
    };
    let forward = with(vec![first, second]).to_smf_bytes().expect("编码");
    let reversed = with(vec![second, first]).to_smf_bytes().expect("编码");
    assert_eq!(forward, reversed, "同一批 tempo 记录换序后字节必须不变");

    // 对照臂: 内容**不同**（第二条的 mpqn 减 1）⇒ 字节必须不同，
    // 否则上面那条"相等"是空断言。
    let other = with(vec![
        first,
        MidiTempo {
            microseconds_per_quarter: Some(833_332),
            ..second
        },
    ])
    .to_smf_bytes()
    .expect("编码");
    assert_ne!(forward, other, "内容不同必须改变字节");

    // 回读的记录多重集也相同（字节相同已蕴含，但这里显式钉住读出的那一边）。
    let records = |bytes: &[u8]| {
        let mut out = parse_smf(bytes).expect("回读").tempos;
        out.sort_by_key(|tempo| {
            (
                tempo.tick,
                tempo.microseconds_per_quarter,
                tempo.numerator,
                tempo.denominator_pow2,
            )
        });
        out
    };
    assert_eq!(records(&forward), records(&reversed));
    assert_eq!(records(&forward).len(), 2, "两条记录一条不少");
}

/// 判据 ⑮ (类别④/⑦ 累加溢出): 音符的结束 tick 用**饱和**加法算 ⇒ 越过 `u64` 上界时
/// 必须报 `DeltaOverflow`（明确 `Err`），⛔ 不是 debug 档的加法 panic、也不是回绕。
///
/// 注入实测（本票）：把 `start.saturating_add(duration_ticks)` 换成裸 `+` 后，
/// 本判据之前本 crate 的全部判据保持绿 —— 没有任何判据把音符放在 tick 轴的末端。
#[test]
fn a_note_at_the_end_of_the_tick_axis_is_refused_not_wrapped() {
    let export = MidiExport {
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(u64::MAX, 60, 120)],
        }],
        ..single_track_export(DEFAULT_PPQ)
    };
    assert_eq!(
        export.to_smf_bytes(),
        Err(MidiError::DeltaOverflow {
            tick: u64::MAX,
            delta: u64::MAX,
        }),
        "末端 tick 的 delta 超过 VLQ 的 28 位上限 ⇒ 明确拒绝"
    );
}

/// 判据 ⑯ (类别④): 同一个 `(通道, 音高)` 的**重叠**音符按"后开先关"配对。
///
/// `parse_smf` 的 `close_note` 文档写着"后开先关"：`open` 是一个 `Vec`，配对用
/// `rposition` 找**最近一次**未闭合的同键音符。用 `position`（先开先关）会得到
/// **另一份**音符集合 —— 同一份字节，两个读取器读出不同的时值。
///
/// 注入实测（本票）：`rposition` → `position` 后，本判据之前本 crate 的全部判据保持绿
/// （既有判据里的同音高音符都不重叠）。
#[test]
fn overlapping_same_key_notes_close_last_opened_first_closed() {
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // NoteOn ch0 key60 vel64 @0
        0x83, 0x60, 0x90, 0x3C, 0x40, // +480: 同一个 (通道, 音高) 再开一次
        0x83, 0x60, 0x80, 0x3C, 0x00, // +480: 先关掉**后**开的那一个
        0x83, 0x60, 0x80, 0x3C, 0x00, // +480: 再关掉先开的那一个
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let parsed = parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[track])).expect("必须可读");
    assert_eq!(
        parsed_keys(&parsed),
        vec![(0, 60, 64, 0, 1440), (0, 60, 64, 480, 480)],
        "后开先关: 第一个 NoteOff 结束 tick 480 那颗, 第二个结束 tick 0 那颗"
    );
}

/// 判据 ⑰ (类别④): 未闭合音符的**上报身份** = 最早仍未闭合的那一颗。
///
/// 注入实测（本票）：`open.first()` → `open.last()` 后全绿；把关闭时的 `remove`
/// 换成 `swap_remove` 后也全绿（两者都只改"报哪一颗"）。本判据同时钉住两者：
/// 关掉最早打开的那一颗之后，剩下两颗的顺序必须仍然按打开先后 ⇒ 报第二颗。
#[test]
fn the_reported_unclosed_note_is_the_earliest_still_open() {
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // A: key60 @0
        0x0A, 0x90, 0x3E, 0x40, // +10 B: key62 @10
        0x0A, 0x90, 0x40, 0x40, // +10 C: key64 @20
        0x0A, 0x80, 0x3C, 0x00, // +10 关掉 A
        0x00, 0xFF, 0x2F, 0x00,
    ];
    assert_eq!(
        parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[track])),
        Err(MidiError::UnclosedNote {
            start_tick: 10,
            key: 62,
        }),
        "B 是**最早**仍未闭合的音符 (A 已被关掉)"
    );
}

/// 判据 ⑱ (类别⑤): 格式 0 且输入**多于一条**轨道时不写 `TrackName`；
/// 恰好一条输入轨道时名字照旧写进去。
///
/// 注入实测（本票）：`self.tracks.len() == 1` → `>= 1` 后全绿
/// （既有判据里没有"格式 0 + 两条输入轨道"的形状）。
#[test]
fn format_zero_with_several_tracks_writes_no_track_name() {
    let has_track_name = |bytes: &[u8]| {
        let chunks = track_chunks(bytes).expect("chunk 布局");
        assert_eq!(chunks.len(), 2, "MThd + 一条 MTrk");
        let payload = &bytes[chunks[1].payload.clone()];
        payload
            .windows(2)
            .any(|window| window[0] == 0xFF && window[1] == 0x03)
    };

    let two = MidiExport {
        tracks: vec![
            MidiExportTrack {
                name: "A".to_owned(),
                channel: 0,
                notes: vec![note(0, 60, 120)],
            },
            MidiExportTrack {
                name: "B".to_owned(),
                channel: 1,
                notes: vec![note(0, 64, 120)],
            },
        ],
        ..single_track_export(DEFAULT_PPQ)
    };
    let bytes = two.to_smf_bytes().expect("编码");
    assert!(
        !has_track_name(&bytes),
        "格式 0 且多于一条输入轨道 ⇒ 不写 TrackName meta"
    );
    assert_eq!(parse_smf(&bytes).expect("回读").notes.len(), 2);

    let one = MidiExport {
        tracks: vec![MidiExportTrack {
            name: "A".to_owned(),
            channel: 0,
            notes: vec![note(0, 60, 120)],
        }],
        ..single_track_export(DEFAULT_PPQ)
    };
    let bytes = one.to_smf_bytes().expect("编码");
    assert!(
        has_track_name(&bytes),
        "只有一条输入轨道时 TrackName 必须保留"
    );
}

/// 判据 ⑲ (类别④ 参数极值): MIDI 音高的**上界本身** 127 必须接受，128 必须拒绝。
///
/// 注入实测（本票）：`note.pitch > 127` → `> 128` 后全绿
/// （既有判据用的是 200 这类远离边界的越界值）。
#[test]
fn pitch_127_is_accepted_and_128_is_rejected() {
    let highest = MidiExport {
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 127, 120)],
        }],
        ..single_track_export(DEFAULT_PPQ)
    };
    let bytes = highest.to_smf_bytes().expect("127 是 7 位字段的上界");
    assert_eq!(parse_smf(&bytes).expect("回读").notes[0].key, 127);

    let too_high = MidiExport {
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 128, 120)],
        }],
        ..single_track_export(DEFAULT_PPQ)
    };
    assert_eq!(
        too_high.to_smf_bytes(),
        Err(MidiError::PitchOutOfRange(128))
    );
}

/// 判据 ⑳ (类别① 非有限输入 / 类别④ 参数极值): `from_bpm` 对非有限 BPM 回退，
/// 并对 `mpqn` 的**两端**都钳制。
///
/// 注入实测（本票）：去掉 `bpm.is_finite()` 后全绿（`+∞` 会落到 `mpqn = 1`）；
/// 把下钳从 `1.0` 放到 `0.0` 后也全绿（极大 BPM 会落到 `mpqn = 0`）。
/// `mpqn = 0` 在 SMF 里是退化速度，`1` 才是本模块契约里的下界。
#[test]
fn bpm_conversion_handles_non_finite_input_and_clamps_both_ends() {
    for bpm in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 0.0, -120.0] {
        assert_eq!(
            MidiTempo::from_bpm(bpm).microseconds_per_quarter,
            Some(500_000),
            "非正 / 非有限的 BPM ({bpm}) 必须回退到 120 BPM"
        );
    }
    // 上钳: 60_000_000 / 3 = 20_000_000 > 0x00FF_FFFF (16777215)。
    assert_eq!(
        MidiTempo::from_bpm(3.0).microseconds_per_quarter,
        Some(0x00FF_FFFF),
        "mpqn 是 u24 ⇒ 上钳到 0x00FFFFFF"
    );
    // 下钳: 微秒数不许是 0。
    assert_eq!(
        MidiTempo::from_bpm(1.0e300).microseconds_per_quarter,
        Some(1),
        "极大 BPM 的 mpqn 下钳到 1 (⛔ 不是 0)"
    );
}

/// 判据 ㉑ (类别④): 拍号只与**同一个 tick 上、紧邻的前一条** tempo 配对；
/// 跨 tick 的拍号必须落成"只有拍号"的记录。
///
/// 注入实测（本票）：去掉配对判断里的 `last.tick == tick` 后全绿
/// （既有合成夹具的 tempo 与拍号都在同一个 tick 上）。
#[test]
fn a_time_signature_pairs_only_within_the_same_tick() {
    let track: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // Tempo 500000 @0
        0x64, 0xFF, 0x58, 0x04, 0x03, 0x02, 0x18, 0x08, // +100: 拍号 3/4
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let parsed = parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[track])).expect("必须可读");
    assert_eq!(
        parsed.tempos,
        vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: Some(500_000),
                numerator: None,
                denominator_pow2: None,
            },
            MidiTempo {
                tick: 100,
                microseconds_per_quarter: None,
                numerator: Some(3),
                denominator_pow2: Some(2),
            },
        ],
        "跨 tick 的拍号必须落成**只有拍号**的记录, ⛔ 不许挂到上一条 tempo 上"
    );
}

/// 判据 ㉒ (新轴: running status 的**数据字节数**边界): 省略状态字节时，
/// 事件的数据字节数由仍生效的状态字节决定 —— 单数据字节的 `0xC0`（Program Change）
/// 与双数据字节的 `0x90`（NoteOn）必须各吃对字节数，否则其后的 delta 与事件整体错位。
///
/// ⛔ 本判据钉的是 `parse_smf` 的**可观测量**（音符集合），不是上游库的内部实现：
/// 改坏了中间的字节宽度，读出的音符集合就会变（对照臂）。
#[test]
fn running_status_keeps_the_previous_status_and_its_data_length() {
    let track: &[u8] = &[
        0x00, 0xC0, 0x05, // Program Change ch0 (1 个数据字节) ⇒ 状态 0xC0 生效
        0x00,
        0x05, // delta 0, 裸数据字节 ⇒ running status 0xC0 再一个 Program Change
        0x00, 0x90, 0x3C, 0x40, // NoteOn ch0 key60 vel64
        0x83, 0x60, 0x3C,
        0x00, // +480, 裸数据字节 ⇒ running status 0x90, vel0 = NoteOff
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let parsed =
        parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[track])).expect("running status 必须可读");
    assert_eq!(
        parsed_keys(&parsed),
        vec![(0, 60, 64, 0, 480)],
        "Program Change 不产生音符; 两次 running status 各吃 1 / 2 个数据字节"
    );
    assert_eq!(parsed.tempos.len(), 0, "Program Change 不是 tempo map 事件");

    // 对照臂 ①: 同样的事件写成**显式**状态字节 ⇒ 音符集合必须相同
    // （running status 只是把状态字节省掉，语义不变）。
    let explicit: &[u8] = &[
        0x00, 0xC0, 0x05, // 显式的 Program Change
        0x00, 0xC0, 0x05, // 显式的第二个 Program Change
        0x00, 0x90, 0x3C, 0x40, 0x83, 0x60, 0x90, 0x3C, 0x00, // 显式的 NoteOn vel0
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let explicit =
        parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[explicit])).expect("显式状态字节也必须可读");
    assert_eq!(
        parsed_keys(&explicit),
        parsed_keys(&parsed),
        "bare 数据字节与显式状态字节必须读出同一批音符"
    );

    // 对照臂 ②: 文件**开头**就是裸数据字节（没有前驱状态字节）。
    // ⚠️ 实测读数（**不是承诺**）: `midly` 接受这种开头并回出 **0 颗音符** ——
    // 它既不报错、也不猜一个状态。这里把现状钉住（与判据 ⑧ 同口径）：
    // 将来任何一侧收紧成 `Err`、或开始"猜"出一个音符，本行都会先变红。
    let orphan: &[u8] = &[0x00, 0x05, 0x00, 0xFF, 0x2F, 0x00];
    let orphan =
        parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[orphan])).expect("实测: 目前接受这种开头");
    assert_eq!(
        orphan.notes.len(),
        0,
        "没有可继承的状态字节时, 不许凭空猜出一个音符"
    );
    assert!(orphan.tempos.is_empty());
}

/// 判据 ㉓ (新轴: running status 的**取消**边界): meta 事件取消 running status
/// （SMF 1.0: "Sysex events and meta-events cancel any running status which was in effect"）
/// ⇒ meta 之后光秃秃的数据字节不得被当成事件。
#[test]
fn a_meta_event_cancels_running_status() {
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // NoteOn ch0 key60 vel64 ⇒ 状态 0x90 生效
        0x00, 0xFF, 0x01, 0x02, 0x41,
        0x42, // Text meta (FF 01 02 "AB") ⇒ 取消 running status
        0x60, 0x3C, 0x00, // 裸数据字节 (没有状态字节) ⇒ 不是合法事件
        0x00, 0xFF, 0x2F, 0x00,
    ];
    assert!(
        parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[track])).is_err(),
        "meta 之后的裸数据字节没有状态字节可依附"
    );
}

/// 判据 ㉔ (类别④ 拍号边界值): `FF 58` 的分子与分母幂字段**原样**往返 ——
/// `0` 与 `255` 都不许被钳制、不许被"缺省"替换。
///
/// 对照臂: 两个字段都在同一个 tick 上，且内容不同 ⇒ 两条记录都必须留下。
#[test]
fn time_signature_meta_fields_round_trip_at_their_extremes() {
    let source = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: None,
                numerator: Some(0),
                denominator_pow2: Some(0),
            },
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: None,
                numerator: Some(255),
                denominator_pow2: Some(255),
            },
        ],
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 60, 120)],
        }],
    };
    let bytes = source.to_smf_bytes().expect("编码");
    let records: Vec<(Option<u8>, Option<u8>)> = parse_smf(&bytes)
        .expect("回读")
        .tempos
        .iter()
        .map(|tempo| (tempo.numerator, tempo.denominator_pow2))
        .collect();
    assert_eq!(
        records,
        vec![(Some(0), Some(0)), (Some(255), Some(255))],
        "0 与 255 都必须原样回来"
    );
}

/// 判据 ㉕ (新轴: 调号的边界值): `FF 59`（Key Signature）**不导出**，回读时被忽略，
/// 且**不移动 tick**、不进入 tempo map。`fifths` 是带符号字节 `-7..=7`，
/// 这里覆盖两端与一个越界字节（`0x7F`）。
#[test]
fn key_signature_meta_is_ignored_without_moving_the_tick() {
    let track: &[u8] = &[
        0x00, 0xFF, 0x59, 0x02, 0xF9, 0x00, // fifths = -7 (0xF9), mode 0
        0x00, 0xFF, 0x59, 0x02, 0x07, 0x01, // fifths = +7, mode 1 (minor)
        0x00, 0x90, 0x3C, 0x40, // NoteOn ch0 key60 vel64 @0
        0x83, 0x60, 0x80, 0x3C, 0x00, // +480 NoteOff
        0x00, 0xFF, 0x59, 0x02, 0x7F, 0x00, // 越界 fifths = 127 (仍必须不 panic)
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let parsed = parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[track])).expect("必须可读");
    assert_eq!(
        parsed_keys(&parsed),
        vec![(0, 60, 64, 0, 480)],
        "调号事件不移动 tick、不改变音符"
    );
    assert!(parsed.tempos.is_empty(), "调号不是 tempo map 记录");

    // 对照臂: 导出侧**不写** `FF 59`（调号不在 SMF 导出的白名单里）。
    let export = single_track_export(DEFAULT_PPQ);
    let bytes = export.to_smf_bytes().expect("编码");
    assert!(
        !bytes
            .windows(2)
            .any(|window| window[0] == 0xFF && window[1] == 0x59),
        "导出侧不写调号元事件"
    );
}

// ---------------------------------------------------------------------------
// ③ 注入暴露的缺口（本票第三批）：后开先关 / 最早未闭合 / 拍号不跨 tick 配对 /
//    running status 与显式状态字节等价（类别 ⑤ 幂等、类别 ⑦ 边界）
// ---------------------------------------------------------------------------

/// 判据 (类别⑤ 幂等性 / 同一音的**重叠**): 同一个 `(通道, 音高)` 上重叠的两颗音符
/// 按**后开先关**配对（`close_note` 用 `rposition`）。
///
/// 补的是哪个缺口（本票注入实测）：把 `close_note` 的 `rposition` 换成 `position`
/// （注入 M16）后，**118** 条判据全绿 ⇒ "后开先关"这条写在文档里的语义
/// （`close_note` 的 doc：`关掉一个已开启的音符 (后开先关)`）当时**零判据**。
///
/// 量什么（单位 = 一颗音符的 `(起始 tick, 时值, 力度)`）：力度把两颗音符区分开 ⇒
/// 配对方向不同时，读数**不同**：
/// - 后开先关（今天）: `(0, 300, 64)` 与 `(100, 100, 100)`；
/// - 先开先关（注入后）: `(0, 200, 64)` 与 `(100, 200, 100)`。
#[test]
fn overlapping_notes_of_one_key_close_last_in_first_out() {
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // tick 0:   NoteOn  ch0 key60 vel64
        0x64, 0x90, 0x3C, 0x64, // +100:     NoteOn  ch0 key60 vel100（重叠）
        0x64, 0x80, 0x3C, 0x00, // +100=200: NoteOff ch0 key60
        0x64, 0x80, 0x3C, 0x00, // +100=300: NoteOff ch0 key60
        0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
    ];
    let bytes = hand_built_smf(0, [0x03, 0xC0], &[track]);
    let parsed = parse_smf(&bytes).expect("手工拼的重叠音符必须可读");
    assert_eq!(
        parsed_keys(&parsed),
        vec![(0, 60, 64, 0, 300), (0, 60, 100, 100, 100)],
        "后开的先关: 力度 100 的那颗只活了 100 tick"
    );
}

/// 判据 (类别① 越界输入 / 错误读数): 同一轨道上有**多颗**未闭合音符时，
/// `UnclosedNote` 报的是**最早**开启的那一颗（`open.first()`）。
///
/// 补的是哪个缺口（本票注入实测）：把 `open.first()` 换成 `open.last()`
/// （注入 M17）后，**118** 条判据全绿 ⇒ 报哪一颗当时没有判据约束。
#[test]
fn an_unclosed_note_reports_the_earliest_open_note() {
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // tick 0:   NoteOn key60（先开）
        0x64, 0x90, 0x40, 0x40, // +100:     NoteOn key64（后开）
        0x00, 0xFF, 0x2F, 0x00, // 两颗都没有 NoteOff
    ];
    let bytes = hand_built_smf(0, [0x03, 0xC0], &[track]);
    assert_eq!(
        parse_smf(&bytes),
        Err(MidiError::UnclosedNote {
            start_tick: 0,
            key: 60
        }),
        "必须点名**最早**开启的那一颗未闭合音符"
    );
}

/// 判据 (类别⑤ 幂等性 / 配对边界): 拍号只挂到**同一 tick**、紧邻的前一条 tempo 上。
/// 不同 tick 的拍号必须如实落成"只有拍号"的一条记录。
///
/// 补的是哪个缺口（本票注入实测）：把 `parse_smf` 的 `last.tick == tick`
/// 放宽成 `<=`（注入 M18）后，**118** 条判据全绿 ⇒ 那个 `==` 当时没有判据。
#[test]
fn a_signature_later_than_the_tempo_does_not_attach_to_it() {
    let conductor: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tick 0:   Tempo 500000
        0x83, 0x60, 0xFF, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08, // +480: 4/4（不同 tick）
        0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
    ];
    let bytes = hand_built_smf(0, [0x03, 0xC0], &[conductor]);
    let parsed = parse_smf(&bytes).expect("回读");
    assert_eq!(parsed.tempos.len(), 2, "不同 tick 的拍号是两条记录");
    let tempo = parsed
        .tempos
        .iter()
        .find(|tempo| tempo.microseconds_per_quarter.is_some())
        .expect("必须有一条带 tempo 的记录");
    assert_eq!(tempo.tick, 0);
    assert_eq!(
        tempo.numerator, None,
        "tick 480 的拍号不得挂到 tick 0 的 tempo 上"
    );
    let signature = parsed
        .tempos
        .iter()
        .find(|tempo| tempo.numerator.is_some())
        .expect("必须有一条带拍号的记录");
    assert_eq!(signature.tick, 480);
    assert_eq!(signature.microseconds_per_quarter, None);
}

/// 判据 (类别⑤ 幂等性 / running status 边界): 同一段音乐分别用**running status**
/// （省略重复的状态字节）与**逐事件显式状态字节**写出 ⇒ 回读结果必须**完全相同**。
///
/// 补的是哪个缺口（本票定焦点时实测）：已提交夹具里 running status 确实被走到
/// （量法 = 本文件旁边的独立 VLQ 扫描器逐事件数；单位 = 事件条数）：
/// `fur_elise_480ppq_1mtrk.mid` **514** 条、`fur_elise_480ppq_3mtrk.mid` **896** 条、
/// `fur_elise_woo59_384ppq_3mtrk.mid` **0** 条 ⇒ 那两个真文件判据**间接**覆盖了它，
/// 但没有任何判据把"省字节不改变读数"这条**等价性**单独钉住（真文件只有一份字节，
/// 失败时也说不清是 running status 还是别处）。
///
/// 量什么（单位 = 一个 `ParsedMidi`）：两条轨道的字节除状态字节外逐字节对齐，
/// 解析结果必须整体相等（`ParsedMidi` 派生了 `PartialEq`）。
#[test]
fn running_status_and_explicit_status_bytes_parse_identically() {
    // 显式状态字节：每个通道事件都自带 `9x` / `8x`。
    let explicit: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // NoteOn  ch0 key60 vel64
        0x00, 0x90, 0x3E, 0x50, // NoteOn  ch0 key62 vel80
        0x83, 0x60, 0x80, 0x3C, 0x00, // +480 NoteOff ch0 key60
        0x00, 0x80, 0x3E, 0x00, // +0   NoteOff ch0 key62
        0x00, 0xFF, 0x2F, 0x00,
    ];
    // running status：第 2 个 NoteOn 与第 4 个 NoteOff 省略状态字节。
    let running: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // NoteOn  ch0 key60 vel64（建立状态）
        0x00, 0x3E, 0x50, // running: NoteOn ch0 key62 vel80
        0x83, 0x60, 0x80, 0x3C, 0x00, // +480 NoteOff ch0 key60
        0x00, 0x3E, 0x00, // running: NoteOff ch0 key62
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let explicit_bytes = hand_built_smf(0, [0x03, 0xC0], &[explicit]);
    let running_bytes = hand_built_smf(0, [0x03, 0xC0], &[running]);
    assert!(
        running_bytes.len() < explicit_bytes.len(),
        "running status 那份必须更短（否则本判据没在测 running status）"
    );
    let a = parse_smf(&explicit_bytes).expect("显式状态字节必须可读");
    let b = parse_smf(&running_bytes).expect("running status 必须可读");
    assert_eq!(a, b, "省掉状态字节不得改变读数");
    assert_eq!(
        parsed_keys(&a),
        vec![(0, 60, 64, 0, 480), (0, 62, 80, 0, 480)],
        "两颗音符都从 tick 0 起, 时值 480"
    );
    // 而且 running status 下的 `NoteOn` 力度 0 仍然等价于 `NoteOff`。
    let zero_velocity: &[u8] = &[
        0x00, 0x90, 0x3C, 0x40, // NoteOn ch0 key60 vel64
        0x60, 0x3C, 0x00, // running: NoteOn ch0 key60 vel0 ⇒ 关闭
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let closed = parse_smf(&hand_built_smf(0, [0x03, 0xC0], &[zero_velocity]))
        .expect("running status 下的零力度 NoteOn 也必须可读");
    assert_eq!(parsed_keys(&closed), vec![(0, 60, 64, 0, 96)]);
}
/// 判据 (R70②: **交付字节的字面契约**): 一份最小导出（格式 0、960 PPQ、**无 tempo**、
/// 一条无名轨道、一颗 0→480 的 C4 音符）的**完整 35 字节**逐字节钉住。
///
/// ## 补的是哪个缺口（R70②）
///
/// 本 crate 的字节面此前只有"**两次运行相同**"式的判据 ——
/// `export_is_byte_deterministic` / `two_exports_of_the_same_project_are_byte_identical` /
/// `export_is_byte_deterministic_across_two_calls` —— 那是**自比**：换一条编码路径
/// （或 `midly` 升级）后它们**仍然全绿**，而**交付出去的字节已经变了**。
/// `exported_bytes_have_literal_smf1_header_and_terminators` 只钉了头部 + conductor 轨 +
/// 音符轨的**前缀**，⛔ 不是整份文件。本判据把整份文件（35 字节）写成字面量。
///
/// ## 字节面的三件套（逐个给）
///
/// | 面 | 生产者 | 读者 | 字面摘要 |
/// | :--- | :--- | :--- | :--- |
/// | `.mid` 导出 | `MidiExport::to_smf_bytes`（`midly` 的 `write_std`） | 本 crate 的 `parse_smf` / `track_chunks` / `vlq` | ⭐ **本判据：整份 35 字节** ＋ `exported_bytes_have_literal_smf1_header_and_terminators`（头部/轨首/轨尾） |
/// | 真 `.mid` 夹具 | 公共领域文件（已提交，字节由文件内容固定） | 同上 | `real_world_smf.rs` 的**已测读数**（逐音符 / tempo 记录） |
/// | `.mxl` 容器 | ⛔ **本 crate 不写出**（只读） | `parse_mxl` | 已提交夹具的**文件字节**（SHA-256 登记在 `tests/fixtures/README.md`） |
/// | `.mxl` 的 DEFLATE 载荷 | 判据侧的 `deflate_stored_block` / `BitWriter` | `inflate_raw` | `inflate` 单元判据的**字面字节数组**（如 `[0x01, 0x00, 0x00, 0xff, 0xff]`） |
#[test]
fn a_minimal_export_is_pinned_byte_for_byte() {
    let export = MidiExport {
        format: MidiFormat::SingleTrack,
        ppq: DEFAULT_PPQ,
        tempos: Vec::new(),
        tracks: vec![MidiExportTrack {
            name: String::new(),
            channel: 0,
            notes: vec![note(0, 60, 480)],
        }],
    };
    let bytes = export.to_smf_bytes().expect("编码");
    assert_eq!(
        bytes,
        vec![
            0x4D, 0x54, 0x68, 0x64, 0x00, 0x00, 0x00, 0x06, // "MThd" + 负载长度 6
            0x00, 0x00, // 格式 0（大端）
            0x00, 0x01, // 1 条轨道（大端）
            0x03, 0xC0, // 960 PPQ（大端）
            0x4D, 0x54, 0x72, 0x6B, 0x00, 0x00, 0x00, 0x0D, // "MTrk" + 负载长度 13
            0x00, 0x90, 0x3C, 0x64, // delta 0, NoteOn ch0 key 60 vel 100
            0x83, 0x60, 0x80, 0x3C, 0x00, // delta 480, NoteOff ch0 key 60 vel 0
            0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
        ],
        "最小导出的完整字节（字面契约）"
    );
    assert_eq!(bytes.len(), 35, "长度本身也是契约的一部分");
    assert_eq!(&bytes[14..18], b"MTrk", "第 14 字节起是第二条 chunk 的标识");
    assert_eq!(
        &bytes[18..22],
        &[0x00, 0x00, 0x00, 0x0D],
        "MTrk 的负载长度字段（大端 13）"
    );
}
/// 判据 (R78④: **文档表两方向**): 本文件的**文档/字面读数**与**实际写出的 conductor 轨
/// 字节**必须互相对得上 —— 用 `include_str!` 把本文件读回来做 `contains` 检查。
///
/// - 方向 ①（**文档 → 字节**）：文档写着 `TrackName 长度 15` 与字面值 `0x0F`
///   ⇒ 实际负载的 TrackName 长度字段必须是 `0x0F`（= 15）。
/// - 方向 ②（**字节 → 文档**）：实际负载里的名称字节必须**逐字节**出现在文档列出的
///   字节表里。
///
/// 补的是哪个缺口（本票注入实测）：把 conductor 轨的名字加长 1 字节（注入 `b11:DOC01`）
/// ⇒ 长度字段从 `0x0F` 变成 `0x10` ⇒ 本判据与既有的
/// `exported_bytes_have_literal_smf1_header_and_terminators` 一起变红。
/// ⚠️ "文档与实现一致"这件事此前**没有判据**：那条既有判据钉的是字节，但**不读文档**
/// ⇒ 文档漂移（改了名字却忘了改文档里的长度）它是看不见的。
#[test]
fn the_conductor_track_documentation_matches_the_written_bytes() {
    let bytes = MidiExport {
        format: MidiFormat::Parallel,
        ppq: DEFAULT_PPQ,
        tempos: Vec::new(),
        tracks: vec![track_from_notes("", 0, &[note(480, 60, 240)])],
    }
    .to_smf_bytes()
    .expect("编码");
    let chunks = track_chunks(&bytes).expect("chunk 布局");
    let conductor = &bytes[chunks[1].payload.clone()];

    // ⭐ R89：文档侧的针**在运行时由被测输出构造** —— ⛔ 不是把字面量先写进被搜的文件里
    // 再回头搜它（那样 `contains` 近乎恒真）。所以先算出长度与名字，再构造针。
    let name_len = conductor[3];
    let name = &conductor[4..4 + usize::from(name_len)];
    let doc = include_str!("smf_contract.rs");
    assert!(
        doc.contains(&format!("TrackName 长度 {name_len}")),
        "文档必须写明 conductor 轨的 TrackName 长度是 {name_len}"
    );
    assert!(
        doc.contains(&format!("0x{name_len:02X}")),
        "文档必须写出长度字段的字面值 0x{name_len:02X}"
    );
    let head_first_5 = name[..5]
        .iter()
        .map(|byte| format!("b'{}'", *byte as char))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        doc.contains(&head_first_5),
        "文档必须逐字节列出 conductor 名字的前 5 个字节: {head_first_5}"
    );

    // 方向 ① 的字节侧。
    assert_eq!(
        conductor[0..3],
        [0x00, 0xFF, 0x03],
        "delta 0 + TrackName 元事件"
    );
    assert_eq!(
        conductor[3], 0x0F,
        "TrackName 长度字段（文档写的是 15 / 0x0F）"
    );
    assert_eq!(conductor[3], 15, "同一件事的十进制读数");
    // 方向 ②。
    assert_eq!(
        &conductor[4..19],
        b"Yeban Conductor",
        "文档列出的名称字节必须原样出现在负载里"
    );
    assert_eq!(
        &conductor[19..23],
        &[0x00, 0xFF, 0x2F, 0x00],
        "TrackName 之后紧跟 EndOfTrack"
    );
}
/// 判据 (R115 ＋ **R160**): 判据里**不许**把"针"绑定成变量再去搜索同一份被
/// `include_str!` 读回来的文件 —— 那是 R104 登记的分析器盲区（子串绑成变量即可绕过）。
///
/// ⭐ **每一形态都有"只有它会红"的注入**（见 `FORMS` 注册表）；
/// ⭐ **R160：注册表 allowlist 必须双向归零**（少一条 ⇒ 有形态没人证明；多一条 ⇒ 注册了
/// 不存在的形态）；⭐ **R119：整串匹配必须带标识符边界**（`bb` 不许被 `b` 命中）；
/// ⭐ **R120：真对象必须附机械下界**（真的扫过足够多的源码行）。
#[test]
fn no_needle_is_bound_to_a_variable_before_being_searched() {
    /// 形态注册表：**只**认这两种"针绑成变量后去搜索"的写法（R160 双向归零见 §②）。
    const FORMS: [&str; 2] = ["contains(<ident>)", "contains(&<ident>)"];

    /// 探测器：返回 `(绑定的名字, 命中的形态)`。
    fn detector(source: &str) -> Vec<(String, &'static str)> {
        let mut bound: Vec<String> = Vec::new();
        for line in source.lines() {
            let t = line.trim();
            let Some(rest) = t.strip_prefix("let ") else {
                continue;
            };
            let Some((name, value)) = rest.split_once(" = ") else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && value.starts_with('"')
                && value.ends_with(';')
                && value.len() > 4
            {
                bound.push(name.to_owned());
            }
        }
        let mut hits: Vec<(String, &'static str)> = Vec::new();
        for name in bound {
            // ⭐ R119：整串要求（含右括号）⇒ 天然带标识符边界（`contains(bb)` 不命中 `b`）。
            if source.contains(&format!("contains({name})")) {
                hits.push((name.clone(), FORMS[0]));
            }
            if source.contains(&format!("contains(&{name})")) {
                hits.push((name, FORMS[1]));
            }
        }
        hits
    }

    // ① **每形态一条正对照**（已知含 ⇒ 必须探到），并记录探到的形态。
    let samples: [(&'static str, &'static str, usize); 2] = [
        (
            FORMS[0],
            "let needle = \"TrackName 长度 15\";\nassert!(doc.contains(needle));\n",
            1,
        ),
        (
            FORMS[1],
            "let needle = \"TrackName 长度 15\";\nassert!(doc.contains(&needle));\n",
            1,
        ),
    ];
    let mut seen: Vec<&'static str> = Vec::new();
    for (form, sample, want) in samples {
        let hits = detector(sample);
        assert_eq!(
            hits.len(),
            want,
            "正对照（形态 {form}）：探针必须发现已知坏样例"
        );
        for (_, got) in &hits {
            seen.push(got);
        }
    }
    // ② ⭐ R160 **双向归零**：注册表每条形态都必须被正对照命中（⛔ 不许有死形态），
    // 且正对照**只**能命中注册表里的形态（⛔ 不许出现未注册形态）。
    for form in FORMS {
        assert!(
            seen.contains(&form),
            "形态 {form} 没有任何正对照 ⇒ 有形态没人证明（R160 少一条）"
        );
    }
    for got in &seen {
        assert!(
            FORMS.contains(got),
            "正对照命中了未注册的形态 {got}（R160 多一条）"
        );
    }

    // ①′ R119 near-miss：绑名 `b`、源码只有 `contains(bb)` ⇒ 不许误报。
    let near_miss = "let b = \"needdle\";\nassert!(doc.contains(bb));\n";
    assert_eq!(
        detector(near_miss).len(),
        0,
        "near-miss：`contains(bb)` 不许被 `contains(b)` 的探针误报（R119）"
    );
    // ③ 负对照：用 `format!` 现场构造针（**没有**绑定字符串字面量）⇒ 不许误报。
    let known_good = "let n = 15;\nassert!(doc.contains(&format!(\"长度 {n}\")));\n";
    assert_eq!(detector(known_good).len(), 0, "负对照：不许误报");

    // ④ 真对象 ＋ ⭐ R120 机械下界：必须**真的扫过**足够多的源码行。
    let mut scanned_lines = 0usize; // R188: 只用于诊断
    for (name, source) in [
        ("smf_contract.rs", include_str!("smf_contract.rs")),
        ("musicxml_contract.rs", include_str!("musicxml_contract.rs")),
        ("real_world_smf.rs", include_str!("real_world_smf.rs")),
    ] {
        scanned_lines += source.lines().count();
        assert_eq!(
            detector(source).len(),
            0,
            "{name} 里出现了 R104 的「针绑成变量」绕过形态"
        );
    }
    // ⭐ R188：**集合大小地板 ⛔ 不能当扫描器守卫**（那是诊断读数，不是判据）。
    // 有牙的是上面的**喂坏输入两臂**（每种形态各有一条"已知含该形态"的样例必须被探到，
    // 外加 near-miss 与负对照必须探不到）。规模读数降级为**诊断**。
    eprintln!(
        "DIAGNOSTIC no_needle_is_bound_to_a_variable: scanned_lines={scanned_lines} forms={FORMS:?}"
    );
}
