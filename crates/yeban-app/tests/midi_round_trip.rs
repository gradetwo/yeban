//! **MIDI 语义往返**：`工程 --export-midi--> SMF --import-midi--> 工程' --export-midi--> SMF'`
//! —— 逐项断言"音符在往返之后还是同一批音符"，并把**不保真的字段**逐条钉住。
//!
//! ## 为什么要有这一条（侦察结论：这是一个真缺口）
//!
//! 在加本文件之前，仓库里已有的覆盖是：
//!
//! | 覆盖 | 在哪 | 它断言了什么 |
//! | :--- | :--- | :--- |
//! | 工程 → 字节 → **SMF 读取面** | `crates/yeban-midi/src/export.rs` 的 `exported_notes_match_the_demo_fixture_note_by_note` 等 | 逐音符 `(ch,key,vel,start,dur)`（**不过导入**） |
//! | 工程 → 字节 → 解析 | `crates/yeban-app/tests/cli_contract.rs` 的 `export_midi_writes_a_parseable_deterministic_smf_from_the_real_binary` | 逐音符 + PPQ + 确定性（**不过导入**） |
//! | 工具回传的字节 == 共享映射的字节 | `crates/yeban-mcp/tests/extension_tools.rs` 的 `export_midi_round_trips_in_memory_through_the_shared_mapping` | 字节相等 + 音符**数** + PPQ（**不过导入**） |
//! | SMF 编解码往返 | `crates/yeban-midi/src/midi.rs` 的 `parallel_export_round_trips_the_same_note_set` 等 | 同一批音符（编解码层，**不碰工程**） |
//! | 导入的映射 | `crates/yeban-app/src/import_midi.rs` 的单元判据 | `ParsedNote ⇒ Op::AddNote`（**喂的是手搓的 `ParsedMidi`，不是导出物**） |
//! | 导出→导入 | `crates/yeban-app/src/cli.rs` 的 `import_midi_adds_the_notes_and_marks_the_origin_as_midi_input` | **只有音符数**（`before × 2`）+ 来源标记 |
//!
//! ⇒ 缺的正是"**导出物真的被导入回工程**，且**逐项**（音高 / 起止 / 时值 / 力度）
//! 与导出侧一致，PPQ 与拍号也对得上"。本文件补这一条，并把有损字段**断言**成损耗
//! （⛔ 不是"声称它们被保留"）。
//!
//! ## 损耗表（**每一行都由下面的判据机械钉住**，⛔ 不是散文）
//!
//! | 字段 | 往返后 | 由哪条断言钉住 |
//! | :--- | :--- | :--- |
//! | 音高 `pitch` / 起止 tick / 时值 `duration` / 力度 `velocity` | ✅ **逐颗保真** | ① `n2 == n1 ⊎ collapse(n1)`（多重集逐项相等） |
//! | 绝对 tick 的**取值** | ✅ 保真（960 PPQ 下 `scale_ticks` 是恒等） | ① 的相等式 + ③ 的 `1920 / 2160 / 2868` |
//! | PPQ | ✅ 保真（工程=文件=960） | ② `ppq == 960` 且 `MThd` 头 `03 C0` |
//! | 拍号 / tempo | ⚠ **由工程携带，而非由文件导入**：导入**不读**文件的 conductor | ④ 6/8 的文件导入 3/4 的工程后，重导出**仍是 3/4** |
//! | **轨归属**（哪条轨） | ❌ **丢失**（只对**导入的副本**）：副本一律落到"第一条非 master 音轨的第一个摆放"；原有轨道各自的音符**一颗不动** | ② `MTrk` 数**仍是 4**、通道 1 仍是 3 颗/通道 0 变 15 颗；③ 目标片段 6 + 9 颗、别的轨一颗不变；③b **按身份**挑出的 9 颗副本正是"落点轨"里的新音符 |
//! | **MIDI 通道** | ❌ **丢失**（只对副本）：`MidiNote` 没有通道字段 ⇒ 副本一律落在通道 0；原件保留各自通道 | ② 副本 `(0, …)` 与原件 `(1, …)` 同时存在且通道集合恰为 `{0, 1}`；③b 身份切分 |
//! | `micro_timing_ticks` | ❌ 字段丢失，但**微时值已并入 tick** ⇒ 位置不丢 | ③ `start_tick == 2868` 且 `micro_timing_ticks == None`；③b 副本逐颗 `micro == None` |
//! | `probability` / `ratchet` / 弯音 / 滑音 | ❌ 导入侧不读（导出侧也不写：导出写"作者写下的音符"，不做触发/连击判定） | ③b：夹具里那颗**确实** `expressive == true`（先断言，否则该行空转），而 9 颗副本逐颗 `expressive == false` |
//! | 歌词 `syllable` | ⚠ ⭐ `#312`：**导出侧真的写**（F1 里有 `FF 05`），**导入侧还没读**（`parse_smf` 忽略 `MetaMessage::Lyric`）⇒ 副本仍 `expressive == false`；这一行**不再**声称"SMF 没有载体" | ① 之后对 F1 的 `FF 05 02 6C 61` 断言 + ③b 的 `!row.expressive` |
//! | 非 960 PPQ 的文件 | ⚠ 按整数比例换算到 960（`scale_ticks`），不能整除时**截断** | 由 `import_midi.rs` 的单元判据覆盖（480 ⇒ ×2），本文件不重复 |
//!
//! ## 规范来源 (Normative)
//!
//! - `[ARCH-FMT-001 §5.5]`：SMF 导出与**严格回读**（回读侧对未闭合音符报错）；
//! - `[MODEL-AST-001]`：960 PPQ 整数时钟 —— 往返的 tick 断言全部是整数相等，⛔ 无浮点；
//! - `[MODEL-AST-005]`：`MidiNote` 的字段语义（`micro_timing_ticks` 等）；
//! - `ADR-0001 D47`：MIDI 导出的唯一出口 = app CLI（本判据走**真二进制**的 CLI，⛔ 不绕旁路）；
//! - `ADR-0001 D28`：注入/映射实现只能有一份 —— 本判据的期望值取自**导出物本身**与
//!   独立的模型侧计数，⛔ 不从实现内部抄。
//!
//! ## 本机 vs CI
//!
//! 本文件要跑**真二进制**（`CARGO_BIN_EXE_yeban-app`，含 Slint 链接），因此它的
//! 权威判决在 CI；本机 `cargo test -p yeban-app` 在缓存的 target/ 上也可以跑（见台账）。
//! ⚠ 它**不**依赖 `midi-input` 特性（那个特性只管物理端口 `--midi-in`）⇒ 两档都跑到同一条判据。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use yeban_app::bridge::demo_project;
use yeban_app::open::open_project_file;
use yeban_midi::midi::{MidiFormat, ParsedMidi, parse_smf, track_chunks};
use yeban_model::container::write_project_container;
use yeban_model::ids::AssetHash;
use yeban_model::{ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote};
use yeban_model::{TimeSignature, TrackKind, YebanProjectV1};

/// 被测二进制的绝对路径（由 Cargo 注入）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 一次子进程执行允许的最长时间（照 `cli_contract.rs` 的口径：⛔ 不许把 CI 挂死）。
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// 一次子进程执行的输出。
#[derive(Debug)]
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn invoke(args: &[&str]) -> Run {
    let mut child = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("无法执行 {BIN}: {error}"));
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("`yeban-app {args:?}` 在 {RUN_TIMEOUT:?} 内没有退出");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let output = child.wait_with_output().expect("收集输出");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-midi-round-trip-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 取某一行里 `key=value` 的 `value`（到下一个空白为止）。
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

fn line_with<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// 把工程写成一个真 `.yeban` 容器（与生产写出路径**同一个**函数）。
fn write_container(dir: &Path, name: &str, project: &YebanProjectV1) -> PathBuf {
    let asset = b"yeban-midi-round-trip-asset".to_vec();
    let hash = AssetHash::of_bytes(&asset);
    let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    assets.insert(hash, asset);
    let bytes = write_project_container(project, b"round-trip-history", &assets).expect("写出容器");
    let path = dir.join(name);
    std::fs::write(&path, &bytes).expect("写容器");
    path
}

/// **非平凡夹具**：在演示工程上再放一块 MIDI 片段（音高 / 时值 / 力度都不同），
/// 并把拍号显式改成 **3/4**（⛔ 不是默认 4/4 ⇒ 拍号断言才有内容）。
///
/// 摆到"第二条 MIDI 轨"上（身份升序；第一条留给导入的落点），摆放起点 **1920**（非零）。
fn round_trip_fixture() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = demo_project();
    project.time_signature = TimeSignature {
        numerator: 3,
        denominator: 4,
    };

    // 第二块 MIDI 片段：音高、时值、力度都与第一块不同；第三颗带微时值 -12。
    let clip_id = EntityId::new();
    let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for (start, pitch, duration, velocity, micro) in [
        (0_u64, 36_u8, 240_u64, 33_u8, None),
        (240, 38, 120, 77, None),
        (960, 41, 960, 127, Some(-12_i16)),
    ] {
        let id = EntityId::new();
        notes.insert(
            id,
            MidiNote {
                velocity,
                micro_timing_ticks: micro,
                ..MidiNote::new(id, start, pitch, duration)
            },
        );
    }
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "RoundTrip B".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );

    // 第二条 **MIDI** 轨（身份升序的第一条是导入落点，⛔ 不动它）。
    let target = project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id && track.kind == TrackKind::Midi)
        .nth(1)
        .map(|track| track.id)
        .expect("演示夹具至少有两条 MIDI 轨");
    let placement_id = EntityId::new();
    project
        .tracks
        .get_mut(&target)
        .expect("轨道在")
        .clips
        .insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: 1920,
                duration_ticks: 3840,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
    assert_eq!(project.validate(), Ok(()), "夹具必须是合法工程");

    // 给**落点片段**里的一颗音符挂上**往返留不住**的表现力字段（概率/连击 +
    // 歌词：⭐ `#312` 起导出侧会写 `FF 05`、导入侧还没读）：
    // 这样"往返后它们为空"就不是空转的断言（夹具侧会先断言它确实是 `true`）。
    let destination_track = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("有一条非 master 音轨");
    let destination_clip = project.tracks[&destination_track]
        .clips
        .values()
        .next()
        .expect("落点轨道有摆放")
        .clip_id;
    if let Some(entry) = project.clip_pool.get_mut(&destination_clip)
        && let ClipContent::Midi { notes } = &mut entry.content
    {
        let note = notes
            .values_mut()
            .next()
            .expect("落点片段必须有音符（演示夹具）");
        note.probability = Some(0.5);
        note.ratchet = Some(2);
        note.syllable = Some("la".to_owned());
    }
    assert_eq!(project.validate(), Ok(()), "加了表现力字段后仍必须合法");
    (project, target, clip_id)
}

/// 模型侧一颗音符的可观测元组 `(pitch, velocity, start_tick, duration_ticks, micro_timing_ticks)`。
type ModelNote = (u8, u8, u64, u64, Option<i16>);

/// 一条轨道上某个片段里的音符（按 tick 排序），用公开结构读回来。
fn clip_notes(project: &YebanProjectV1, track: EntityId, clip: EntityId) -> Vec<ModelNote> {
    let mut out: Vec<ModelNote> = project.tracks[&track]
        .clips
        .values()
        .filter(|placement| placement.clip_id == clip)
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values())
        .map(|note| {
            (
                note.pitch,
                note.velocity,
                note.start_tick,
                note.duration_ticks,
                note.micro_timing_ticks,
            )
        })
        .collect();
    out.sort_unstable();
    out
}

/// 模型侧一颗音符的完整可观测行（**含身份** —— 用它把"原有的"与"导入的副本"分开）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NoteRow {
    id: EntityId,
    pitch: u8,
    velocity: u8,
    start: u64,
    duration: u64,
    micro: Option<i16>,
    /// 往返后**留不住**的表现力字段（概率 / 连击 / 弯音 / 滑音：导入侧不读；歌词：
    /// ⭐ `#312` **导出侧会写** `FF 05`、导入侧还没读）：任一非默认即 `true`。
    expressive: bool,
}

/// 一条轨道上某个片段里的音符行（按 `(pitch, start)` 排序）。
fn clip_rows(project: &YebanProjectV1, track: EntityId, clip: EntityId) -> Vec<NoteRow> {
    let mut rows: Vec<NoteRow> = project.tracks[&track]
        .clips
        .values()
        .filter(|placement| placement.clip_id == clip)
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values())
        .map(|note| NoteRow {
            id: note.id,
            pitch: note.pitch,
            velocity: note.velocity,
            start: note.start_tick,
            duration: note.duration_ticks,
            micro: note.micro_timing_ticks,
            expressive: note.probability.is_some()
                || note.ratchet.is_some()
                || note.slide.is_some()
                || !note.pitch_bend_curve.is_empty()
                || note.syllable.is_some()
                || !note.phonemes.is_empty(),
        })
        .collect();
    rows.sort_unstable_by_key(|row| (row.pitch, row.start));
    rows
}

/// `after` 里**身份不在** `before` 中的那些行 —— 即本次导入**新建**的音符。
fn imported_rows(before: &[NoteRow], after: &[NoteRow]) -> Vec<NoteRow> {
    after
        .iter()
        .filter(|row| !before.iter().any(|known| known.id == row.id))
        .cloned()
        .collect()
}

/// 回读一个 SMF 的音符多重集 `(channel, key, velocity, start, duration)`（升序）。
fn note_multiset(bytes: &[u8]) -> Vec<(u8, u8, u8, u64, u64)> {
    let parsed = parse_smf(bytes).expect("导出的字节必须能被 SMF 读取面读回");
    let mut keys: Vec<(u8, u8, u8, u64, u64)> =
        parsed.notes.iter().map(|note| note.key()).collect();
    keys.sort_unstable();
    keys
}

fn parsed(bytes: &[u8]) -> ParsedMidi {
    parse_smf(bytes).expect("回读")
}

/// **判据（本线新增，端到端语义往返）**：非平凡工程 ⇒ `--export-midi` ⇒
/// `--import-midi` ⇒ `--export-midi` ⇒ **逐项**断言音符不变；有损字段按损耗表**断言**。
#[test]
fn export_import_export_round_trips_every_note_item_by_item() {
    let dir = scratch_dir("round-trip");
    let (project, second_track, second_clip) = round_trip_fixture();
    let fixture = write_container(&dir, "fixture.yeban", &project);
    let fixture_bytes_before = std::fs::read(&fixture).expect("读夹具");
    let fixture_arg = fixture.to_str().expect("utf8");

    // 目的地 = 第一条非 master 音轨的第一个摆放（导入的落点，也是导入判据的口径）。
    let destination_track = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("有一条非 master 音轨");
    let destination_clip = project.tracks[&destination_track]
        .clips
        .values()
        .next()
        .expect("有摆放")
        .clip_id;
    let original_destination_notes =
        clip_notes(&project, destination_track, destination_clip).len();
    assert_eq!(
        original_destination_notes, 6,
        "演示夹具的落点片段有 6 颗音符"
    );

    // ── ① 导出 ────────────────────────────────────────────────────────────
    let f1 = dir.join("F1.mid");
    let run = invoke(&[
        "--open",
        fixture_arg,
        "--export-midi",
        f1.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "导出必须成功; stderr={}", run.stderr);
    let bytes1 = std::fs::read(&f1).expect("读 F1");
    let n1 = note_multiset(&bytes1);

    // ⭐ `#312`：导出侧**真的**把歌词写进文件 —— 夹具给落点片段第一颗音符挂了
    // `syllable = "la"`。因此下面"导入副本 expressive == false"只证明**导入侧
    // 还没读 lyric meta**，⛔ 不再（错误地）证明"SMF 里没有载体"。
    let lyric = [0xFF_u8, 0x05, 0x02, b'l', b'a'];
    assert_eq!(
        bytes1
            .windows(lyric.len())
            .filter(|window| *window == lyric)
            .count(),
        1,
        "F1 里必须恰好一条 `FF 05 02 6C 61`（`la`）"
    );

    // 夹具的非平凡性 + 导出侧的 PPQ / 拍号（**独立**于实现的期望值）。
    assert_eq!(
        n1.len(),
        9,
        "夹具 = 演示 6 颗 + 新增 3 颗 ⇒ 本判据需同步; n1={n1:?}"
    );
    let mut keys: Vec<u8> = n1.iter().map(|note| note.1).collect();
    keys.dedup();
    assert!(keys.len() >= 2, "至少有 2 个不同音高: {n1:?}");
    let mut durations: Vec<u64> = n1.iter().map(|note| note.4).collect();
    durations.sort_unstable();
    durations.dedup();
    assert!(
        durations.len() >= 2,
        "至少有 2 种不同时值（非平凡）: {n1:?}"
    );
    let mut channels: Vec<u8> = n1.iter().map(|note| note.0).collect();
    channels.sort_unstable();
    channels.dedup();
    assert_eq!(channels, vec![0, 1], "两条 MIDI 轨 ⇒ 两个通道: {n1:?}");
    assert_eq!(
        track_chunks(&bytes1).expect("chunk").len(),
        4,
        "MThd + conductor + 2 轨"
    );

    // ── ② 导入 + 重导出（同一次调用里：导入 → 导出 → 保存；顺序是 CLI 的契约） ──
    let f2 = dir.join("F2.mid");
    let saved = dir.join("after-import.yeban");
    let run = invoke(&[
        "--open",
        fixture_arg,
        "--import-midi",
        f1.to_str().expect("utf8"),
        "--export-midi",
        f2.to_str().expect("utf8"),
        "--save-as",
        saved.to_str().expect("utf8"),
    ]);
    assert_eq!(
        run.code, 0,
        "导入 + 导出 + 保存必须成功; stderr={}",
        run.stderr
    );
    let imported = line_with(&run.stdout, "imported-midi:").expect("必须有 imported-midi 行");
    assert_eq!(
        field(imported, "notes").as_deref(),
        Some("9"),
        "导入条数必须等于文件里的音符数: {imported}"
    );
    assert!(
        imported.contains("origin=midi-input"),
        "来源必须是 midi-input: {imported}"
    );
    let exported = line_with(&run.stdout, "exported-midi:").expect("必须有 exported-midi 行");
    assert_eq!(
        field(exported, "notes").as_deref(),
        Some("18"),
        "导出发生在导入**之后**（6+3 原有 + 9 导入 = 18）: {exported}"
    );

    let bytes2 = std::fs::read(&f2).expect("读 F2");
    let n2 = note_multiset(&bytes2);

    // ── ①  逐项等值：`n2 == n1 ⊎ collapse(n1)`（多重集相等 ⇒ 每颗音符都在） ──
    let mut expected = n1.clone();
    expected.extend(
        n1.iter()
            .map(|&(_, key, velocity, start, duration)| (0_u8, key, velocity, start, duration)),
    );
    expected.sort_unstable();
    assert_eq!(
        n2, expected,
        "往返后 = 原有 ⊎ 导入的副本（音高/力度/起止/时值逐项相同；导入副本的通道全为 0）"
    );
    assert_eq!(n2.len(), 18, "音符数恰好翻倍: 9 + 9");

    // ── ② PPQ / 拍号 / tempo（往返不许把它们换掉） ──
    let p1 = parsed(&bytes1);
    let p2 = parsed(&bytes2);
    assert_eq!(p1.ppq, 960, "导出侧 PPQ = 工程的 960 [MODEL-AST-001]");
    assert_eq!(p2.ppq, 960, "往返后 PPQ 仍是 960");
    assert_eq!(&bytes2[12..14], &[0x03, 0xC0], "0x03C0 = 960（大端）");
    for parsed_file in [&p1, &p2] {
        assert_eq!(parsed_file.format, MidiFormat::Parallel, "恒为 SMF 1");
        assert_eq!(parsed_file.tempos.len(), 1, "顶层 bpm ⇒ 一条 tempo");
        assert_eq!(
            parsed_file.tempos[0].microseconds_per_quarter,
            Some(500_000),
            "120 BPM 往返不变"
        );
        assert_eq!(parsed_file.tempos[0].numerator, Some(3), "拍号 3/4（分子）");
        assert_eq!(
            parsed_file.tempos[0].denominator_pow2,
            Some(2),
            "拍号 3/4（分母 4 ⇒ dd=2）"
        );
    }

    // ── ② 损耗：导入的**副本**丢掉轨归属与通道（第二条轨自己的音符一颗不动） ──
    assert_eq!(
        track_chunks(&bytes2).expect("chunk").len(),
        4,
        "两条轨都还有音符 ⇒ 仍是 MThd + conductor + 2 条 MTrk"
    );
    assert_eq!(
        n2.iter().filter(|note| note.0 == 1).count(),
        3,
        "第二条 MIDI 轨自己的 3 颗音符原样留在通道 1: {n2:?}"
    );
    assert_eq!(
        n2.iter().filter(|note| note.0 == 0).count(),
        15,
        "第一条轨 = 原有 6 颗 + 导入的 9 颗（全部堆在同一个落点上）: {n2:?}"
    );
    // 这三颗**来自第二条轨**（文件里是通道 1）：原件留在通道 1，副本落在通道 0。
    for note in [(36_u8, 33_u8, 240_u64), (38, 77, 120), (41, 127, 960)] {
        let (key, velocity, duration) = note;
        let (start, duration) = match key {
            36 => (1920_u64, duration),
            38 => (2160, duration),
            _ => (2868, duration),
        };
        assert!(
            n2.contains(&(1, key, velocity, start, duration)),
            "原件必须留在自己的通道 1: key={key} {n2:?}"
        );
        assert!(
            n2.contains(&(0, key, velocity, start, duration)),
            "损耗：副本丢掉了轨归属/通道 ⇒ 落在通道 0: key={key} {n2:?}"
        );
    }
    assert!(
        n2.iter().all(|note| note.0 <= 1),
        "不许冒出第三条通道: {n2:?}"
    );

    // ── ③ 落盘工程侧的断言（公开入口读回，⛔ 不碰内部字段） ──
    let after = open_project_file(&saved).expect("保存的工程必须能读回");
    let landed = clip_notes(&after, destination_track, destination_clip);
    assert_eq!(
        landed.len(),
        original_destination_notes + n1.len(),
        "损耗的**形状**：全部导入音符落在同一个摆放上（{original_destination_notes} + 9）"
    );
    assert!(
        landed.iter().any(|note| note.2 == 2868 && note.4.is_none()),
        "绝对 tick 保真（1920 + 960 - 12 = 2868），但 `micro_timing_ticks` 字段丢失（None）: {landed:?}"
    );
    for (pitch, velocity) in [(36_u8, 33_u8), (38, 77), (41, 127)] {
        assert!(
            landed
                .iter()
                .any(|note| note.0 == pitch && note.1 == velocity),
            "音高 {pitch} / 力度 {velocity} 必须原样落进工程: {landed:?}"
        );
    }
    assert!(
        landed.iter().all(|note| note.2 != 1920 || note.4.is_none()),
        "导入的音符一律没有微时值: {landed:?}"
    );
    // 第二条 MIDI 轨**一颗都没被改**（导入只动落点）。
    assert_eq!(
        clip_notes(&after, second_track, second_clip),
        vec![
            (36, 33, 0, 240, None),
            (38, 77, 240, 120, None),
            (41, 127, 960, 960, Some(-12)),
        ],
        "别的轨道必须原样（⛔ 导入不是'把整个工程洗一遍'）"
    );
    // ⛔ 导入是**内存里**的编辑：输入容器一个字节都不许被改写。
    assert_eq!(
        std::fs::read(&fixture).expect("再读夹具"),
        fixture_bytes_before,
        "导入不许改写输入容器"
    );

    // ── ③b 按**身份**把导入的副本挑出来，逐项对比（比 SMF 层的多重集更直接） ──
    let before_rows = clip_rows(&project, destination_track, destination_clip);
    assert!(
        before_rows.iter().any(|row| row.expressive),
        "夹具必须带一颗往返留不住的表现力音符（否则下面那条损耗断言是空转的）: {before_rows:?}"
    );
    let after_rows = clip_rows(&after, destination_track, destination_clip);
    let imported = imported_rows(&before_rows, &after_rows);
    assert_eq!(
        imported.len(),
        n1.len(),
        "按身份认出的副本必须恰好是文件里的 {} 颗（⛔ 不是按数值猜的）",
        n1.len()
    );
    let mut copy_tuples: Vec<(u8, u8, u64, u64)> = imported
        .iter()
        .map(|row| (row.pitch, row.velocity, row.start, row.duration))
        .collect();
    copy_tuples.sort_unstable();
    let mut file_tuples: Vec<(u8, u8, u64, u64)> = n1
        .iter()
        .map(|&(_, key, velocity, start, duration)| (key, velocity, start, duration))
        .collect();
    file_tuples.sort_unstable();
    assert_eq!(
        copy_tuples, file_tuples,
        "逐项：副本的音高/力度/起止/时值必须与文件逐颗相同"
    );
    assert!(
        imported.iter().all(|row| row.micro.is_none()),
        "SMF 没有微时值字段 ⇒ 副本的 `micro_timing_ticks` 恒为 None: {imported:?}"
    );
    assert!(
        imported.iter().all(|row| !row.expressive),
        "副本上的表现力字段必须为空：概率/连击/弯音/滑音是导出侧不写，歌词是\
         ⭐ `#312` 后**导出侧写了、导入侧还没读**（`parse_smf` 忽略 lyric meta）: {imported:?}"
    );
    // 对照：原有那颗**带**表现力字段的音符必须原样还在（⛔ 导入不是"把整个工程洗一遍"）。
    assert!(
        after_rows
            .iter()
            .any(|row| row.expressive && before_rows.iter().any(|before| before.id == row.id)),
        "原有的表现力音符必须原样保留: {after_rows:?}"
    );

    eprintln!(
        "[midi-round-trip] n1={} n2={} ppq={}/*{}*/ 头={:02X}{:02X} channels={:?} 落点={} 颗 \
         （按身份认出的副本={} 颗，全部 micro=None 且 expressive=false；原有表现力音符 {} 颗仍在）",
        n1.len(),
        n2.len(),
        p1.ppq,
        p2.ppq,
        bytes2[12],
        bytes2[13],
        channels,
        landed.len(),
        imported.len(),
        after_rows.iter().filter(|row| row.expressive).count()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **判据（损耗表第 4 行）**：导入**不读**文件的 conductor（tempo / 拍号）。
///
/// 做法：造一份 6/8、90 BPM 的文件导入 3/4、120 BPM 的工程 ⇒ 重导出的文件必须
/// **仍是工程的 3/4 / 120 BPM**（⛔ 不是文件里的 6/8 / 90 BPM）。
/// 这一条把"拍号/tempo 由工程携带"这句话变成可执行的断言，而不是散文。
#[test]
fn the_import_ignores_the_files_conductor_metadata() {
    let dir = scratch_dir("conductor");
    let (project, _track, _clip) = round_trip_fixture();
    let fixture = write_container(&dir, "fixture.yeban", &project);
    let fixture_arg = fixture.to_str().expect("utf8");

    // 6/8 + 90 BPM 的**另一个**工程 ⇒ 导出文件里的 conductor 与上面那份不同。
    let mut other = demo_project();
    other.time_signature = TimeSignature {
        numerator: 6,
        denominator: 8,
    };
    other.bpm = 90.0;
    let other_container = write_container(&dir, "other.yeban", &other);
    let f3 = dir.join("F3.mid");
    let run = invoke(&[
        "--open",
        other_container.to_str().expect("utf8"),
        "--export-midi",
        f3.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let p3 = parsed(&std::fs::read(&f3).expect("读 F3"));
    assert_eq!(p3.tempos[0].numerator, Some(6), "F3 的拍号分子");
    assert_eq!(p3.tempos[0].denominator_pow2, Some(3), "6/8 ⇒ dd=3");
    assert_eq!(
        p3.tempos[0].microseconds_per_quarter,
        Some(666_667),
        "90 BPM ⇒ 666667 µs"
    );

    // 导入 3/4 + 120 BPM 的工程 ⇒ 重导出必须**仍是工程的** conductor。
    let f4 = dir.join("F4.mid");
    let run = invoke(&[
        "--open",
        fixture_arg,
        "--import-midi",
        f3.to_str().expect("utf8"),
        "--export-midi",
        f4.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "stderr={}", run.stderr);
    let bytes4 = std::fs::read(&f4).expect("读 F4");
    let p4 = parsed(&bytes4);
    assert_eq!(
        p4.tempos[0].numerator,
        Some(3),
        "损耗：文件的 6/8 被忽略（工程的 3/4 胜出）"
    );
    assert_eq!(p4.tempos[0].denominator_pow2, Some(2), "3/4 ⇒ dd=2");
    assert_eq!(
        p4.tempos[0].microseconds_per_quarter,
        Some(500_000),
        "损耗：文件的 90 BPM 被忽略（工程的 120 BPM 胜出）"
    );
    // 而**音符**确实导进来了（损耗只发生在 conductor，⛔ 不是整条链路没做事）。
    assert_eq!(p4.ppq, 960);
    assert_eq!(
        p4.notes.len(),
        9 + p3.notes.len(),
        "工程原有 9 颗 + 文件里的 {} 颗",
        p3.notes.len()
    );

    eprintln!(
        "[midi-round-trip/conductor] F3={}/{:?}/{:?} ⇒ F4={}/{:?}/{:?}（文件的 conductor 被忽略，音符 {} 颗导进来了）",
        p3.tempos[0]
            .numerator
            .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        p3.tempos[0].denominator_pow2,
        p3.tempos[0].microseconds_per_quarter,
        p4.tempos[0]
            .numerator
            .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        p4.tempos[0].denominator_pow2,
        p4.tempos[0].microseconds_per_quarter,
        p4.notes.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
