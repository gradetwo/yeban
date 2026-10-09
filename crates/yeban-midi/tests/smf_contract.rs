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
