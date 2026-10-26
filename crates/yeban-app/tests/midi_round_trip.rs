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
//! | `probability` / `ratchet` / 弯音 / 滑音 | ❌ 导入侧不读（导出侧也不写：导出写"作者写下的音符"，不做触发/连击判定） | ③b：夹具里那颗**确实** `expressive == true`（先断言，否则该行空转），而 9 颗副本逐颗 `expressive == false`（歌词已单列） |
//! | 歌词 `syllable` | ✅ ⭐ `#317`：**唯一才归属** —— 按文件 tick，恰 1 颗**起点**音符 ∧ 恰 1 条非空严格 UTF-8 词 ⇒ 写进那颗音符；和弦 / 一 tick 多词**不猜**，逐条报 `dropped`；空 `FF 05` 与非 UTF-8 分别计数（⛔ lossy） | ① 之后对 F1 的 `FF 05 02 6C 61` 断言；③b **恰 1 颗**副本 `start == 0` 且 `syllable == Some("la")`，其余 None；⑤ filled 端到端 4 次 `"syllable"` / 4 条 `FF 05`；⑥ 和弦正对照 4 颗 4 词 ⇒ `dropped=4`、0 归属 |
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
use yeban_midi::midi::{
    MidiAuxEventKind, MidiFormat, ParsedLyric, ParsedMidi, parse_smf, track_chunks,
};
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

/// 数一段字节里出现某个子序列多少次（**单位 = 匹配次数**，⛔ 不是行数）。
fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// 读 `.yeban` 容器里的**原始** `project.json` 字节（⛔ 不经过反序列化 ⇒ 可数字节出现次数）。
fn project_json_of(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("读容器");
    let archive =
        yeban_model::container::read_container(&bytes, &Default::default()).expect("容器必须可读");
    archive
        .get("project.json")
        .expect("容器必须有 project.json")
        .data
        .clone()
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

/// ⭐ `#317` 和弦夹具：清掉落点片段的全部音符，只留 **tick 960 的 4 颗**（音高
/// `64/67/72/76`），并按 `syllables` 给它们挂词。⛔ 不依赖别的轨道有音符
///（`demo_project` 只有这一个 MIDI 片段）。
fn lyric_chord_project(syllables: [Option<&str>; 4]) -> YebanProjectV1 {
    let mut project = demo_project();
    let track_id = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("有一条非 master 音轨");
    let clip_id = project.tracks[&track_id]
        .clips
        .values()
        .next()
        .expect("落点轨有摆放")
        .clip_id;
    let entry = project.clip_pool.get_mut(&clip_id).expect("片段在池里");
    let ClipContent::Midi { notes } = &mut entry.content else {
        panic!("演示夹具的落点必须是 MIDI 片段");
    };
    notes.clear();
    for (pitch, syllable) in [64_u8, 67, 72, 76].into_iter().zip(syllables) {
        let id = EntityId::new();
        let mut note = MidiNote::new(id, 960, pitch, 480);
        note.syllable = syllable.map(str::to_owned);
        assert!(notes.insert(id, note).is_none(), "身份必须唯一");
    }
    assert_eq!(project.validate(), Ok(()), "和弦夹具必须合法");
    project
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
    /// ⭐ `#317`：导入侧现在会读 `FF 05` —— 这个字段是**唯一归属**的结果，
    /// 与"留不住的表现力字段"分开量（否则"歌词读到了"会被误读成"回归"）。
    syllable: Option<String>,
    /// 往返后**留不住**的表现力字段（概率 / 连击 / 弯音 / 滑音：导入侧不读；
    /// ⛔ **不含**歌词：`syllable` 单独占一栏）。
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
            syllable: note.syllable.clone(),
            expressive: note.probability.is_some()
                || note.ratchet.is_some()
                || note.slide.is_some()
                || !note.pitch_bend_curve.is_empty()
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

    // ⭐ `#312`/`#317`：导出侧**真的**把歌词写进文件，导入侧现在**也读** —— 夹具给
    // 落点片段第一颗音符挂了 `syllable = "la"`，F1 里就恰好一条 `FF 05 02 6C 61`；
    // 导入后它必须**唯一归属**回 tick 0 的那颗副本（③b 断言）。
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

    // ── ①  逐项等值：`n2 == n1 ⊎ n1`（多重集相等 ⇒ 每颗音符都在，通道完整保留） ──
    let mut expected = n1.clone();
    expected.extend(n1.clone());
    expected.sort_unstable();
    assert_eq!(
        n2, expected,
        "往返后 = 原有 ⊎ 导入的副本（音高/力度/起止/时值/通道逐项相同）"
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

    // ── ② 损耗：导入的**副本**按轨拆分保留（每个非主总线轨道独立落盘），音符**通道完整保留** ──
    assert_eq!(
        track_chunks(&bytes2).expect("chunk").len(),
        5,
        "两条轨都还有音符 + 导入拆分新轨 ⇒ 仍是 MThd + conductor + 3 条 MTrk"
    );
    assert_eq!(
        n2.iter().filter(|note| note.0 == 1).count(),
        6,
        "第二条 MIDI 轨自己的 3 颗音符 + 导入的 3 颗通道 1 副本保留在通道 1: {n2:?}"
    );
    assert_eq!(
        n2.iter().filter(|note| note.0 == 0).count(),
        12,
        "第一条轨原有 6 颗 + 导入的 6 颗通道 0 副本保留在通道 0: {n2:?}"
    );
    // 这三颗来自第二条轨（文件里是通道 1）：原件和副本均在通道 1。
    for note in [(36_u8, 33_u8, 240_u64), (38, 77, 120), (41, 127, 960)] {
        let (key, velocity, duration) = note;
        let (start, duration) = match key {
            36 => (1920_u64, duration),
            38 => (2160, duration),
            _ => (2868, duration),
        };
        assert_eq!(
            n2.iter()
                .filter(|n| **n == (1, key, velocity, start, duration))
                .count(),
            2,
            "原件与副本均完整保留在通道 1: key={key} {n2:?}"
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
        original_destination_notes + 6,
        "损耗的**形状**：落点音轨摆放保留原有 6 颗 + 导入的第 1 轨 6 颗音符（第 2 轨 3 颗按轨拆分到新轨）"
    );
    assert!(
        landed.iter().all(|note| note.2 != 1920 || note.4.is_none()),
        "导入的音符一律没有微时值: {landed:?}"
    );

    // 多轨拆分创建的新音轨包含第 2 轨的 3 颗音符（含绝对 tick 保真与 micro_timing_ticks 丢失）
    let split_track = *after
        .tracks
        .keys()
        .find(|id| !project.tracks.contains_key(id))
        .expect("多轨拆分必须创建了新音轨");
    let split_clip = after.tracks[&split_track]
        .clips
        .values()
        .next()
        .expect("新音轨必须有摆放")
        .clip_id;
    let split_notes = clip_notes(&after, split_track, split_clip);
    assert_eq!(
        split_notes.len(),
        3,
        "第二条音轨的 3 颗音符按轨拆分进入新音轨: {split_notes:?}"
    );
    assert!(
        split_notes
            .iter()
            .any(|note| note.2 == 2868 && note.4.is_none()),
        "绝对 tick 保真（1920 + 960 - 12 = 2868），但 `micro_timing_ticks` 字段丢失（None）: {split_notes:?}"
    );
    for (pitch, velocity) in [(36_u8, 33_u8), (38, 77), (41, 127)] {
        assert!(
            split_notes
                .iter()
                .any(|note| note.0 == pitch && note.1 == velocity),
            "音高 {pitch} / 力度 {velocity} 必须在拆分音轨原样落进工程: {split_notes:?}"
        );
    }
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
        6,
        "落点音轨按身份认出的第 1 轨副本必须恰好是 6 颗（第 2 轨 3 颗拆分到新轨）"
    );
    let mut copy_tuples: Vec<(u8, u8, u64, u64)> = imported
        .iter()
        .map(|row| (row.pitch, row.velocity, row.start, row.duration))
        .collect();
    copy_tuples.sort_unstable();
    let mut file_tuples_track1: Vec<(u8, u8, u64, u64)> = n1
        .iter()
        .filter(|note| note.0 == 0)
        .map(|&(_, key, velocity, start, duration)| (key, velocity, start, duration))
        .collect();
    file_tuples_track1.sort_unstable();
    assert_eq!(
        copy_tuples, file_tuples_track1,
        "逐项：落点音轨副本的音高/力度/起止/时值必须与文件第 1 轨逐颗相同"
    );

    // 拆分音轨上的 3 颗副本与文件第 2 轨逐颗相同
    let split_rows = clip_rows(&after, split_track, split_clip);
    let mut split_tuples: Vec<(u8, u8, u64, u64)> = split_rows
        .iter()
        .map(|row| (row.pitch, row.velocity, row.start, row.duration))
        .collect();
    split_tuples.sort_unstable();
    let mut file_tuples_track2: Vec<(u8, u8, u64, u64)> = n1
        .iter()
        .filter(|note| note.0 == 1)
        .map(|&(_, key, velocity, start, duration)| (key, velocity, start, duration))
        .collect();
    file_tuples_track2.sort_unstable();
    assert_eq!(
        split_tuples, file_tuples_track2,
        "逐项：拆分音轨副本的音高/力度/起止/时值必须与文件第 2 轨逐颗相同"
    );
    assert!(
        imported.iter().all(|row| row.micro.is_none()),
        "SMF 没有微时值字段 ⇒ 副本的 `micro_timing_ticks` 恒为 None: {imported:?}"
    );
    assert!(
        imported.iter().all(|row| !row.expressive),
        "副本上的**非歌词**表现力字段必须为空（概率/连击/弯音/滑音是导出侧不写、\
         导入侧不读）: {imported:?}"
    );
    // ⭐ `#317`：歌词**唯一才归属** —— F1 只有一条 `la`（tick 0，该 tick 恰 1 颗起点音符）
    // ⇒ 6 颗副本里**恰 1 颗**拿到它，其余仍是 None（⛔ 猜、⛔ 复制）。
    let lyric_copies: Vec<&NoteRow> = imported
        .iter()
        .filter(|row| row.syllable.is_some())
        .collect();
    assert_eq!(
        lyric_copies.len(),
        1,
        "F1 的唯一一条歌词只能归属一颗副本: {imported:?}"
    );
    assert_eq!(lyric_copies[0].start, 0, "歌词必须落在同 tick 的副本上");
    assert_eq!(
        lyric_copies[0].syllable.as_deref(),
        Some("la"),
        "文本必须逐字保真: {imported:?}"
    );
    assert!(
        imported
            .iter()
            .all(|row| row.syllable.is_none() || row.start == 0),
        "别的副本不许拿到音节: {imported:?}"
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
         （按身份认出的副本={} 颗，micro=None、非歌词 expressive=false、歌词唯一归属 {} 颗；\
         原有表现力音符 {} 颗仍在）",
        n1.len(),
        n2.len(),
        p1.ppq,
        p2.ppq,
        bytes2[12],
        bytes2[13],
        channels,
        landed.len(),
        imported.len(),
        lyric_copies.len(),
        after_rows.iter().filter(|row| row.expressive).count()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// ⭐ `#317` **C6 主判据（端到端）**：`filled` 导出 **143 B** ⇒ 导入 **default** 工程
/// ⇒ `project.json` 里 `"syllable"` **4 次** ⇒ 重导出 **4 条 `FF 05`**，且
/// `(tick, text)` 逐字 = `(0,do)(960,re)(1908,mi)(2880,fa)`（`1908` 含 `micro -12`）。
///
/// ⚠ **落点必须是 default(demo) 工程**：导入 filled 工程会读到**原件**的 4 个
/// `"syllable"`（导入件一个都没有）⇒ 那就成了一个会自己满足自己的夹具。
#[test]
fn filled_export_imports_its_four_lyrics_and_reexports_them_at_the_same_ticks() {
    let dir = scratch_dir("lyrics-filled");
    let filled = dir.join("Filled.mid");
    let run = invoke(&[
        "--project-sample",
        "filled",
        "--export-midi",
        filled.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "filled 导出必须成功; stderr={}", run.stderr);
    let bytes = std::fs::read(&filled).expect("读 filled.mid");
    assert_eq!(
        bytes.len(),
        168,
        "filled 导出的字节契约是 168 B (含 tempo_map 与 CC 7 音量点)"
    );
    assert_eq!(
        AssetHash::of_bytes(&bytes).as_str(),
        "8643d792aa5b0ba1aa419f0b2799084a0749bf5454cb8b22b5d474b78e7eb265",
        "导出侧一个字节都不许变（#312 的 143 B 契约 + tempo map 16 B + CC 7 9 B）"
    );
    assert_eq!(
        occurrences(&bytes, &[0xFF, 0x05]),
        4,
        "filled 导出里有 4 条 FF 05（夹具非空转）"
    );
    let exported = parsed(&bytes);
    assert_eq!(
        exported.lyrics,
        vec![
            ParsedLyric {
                tick: 0,
                text: "do".to_owned(),
            },
            ParsedLyric {
                tick: 960,
                text: "re".to_owned(),
            },
            ParsedLyric {
                tick: 1908,
                text: "mi".to_owned(),
            },
            ParsedLyric {
                tick: 2880,
                text: "fa".to_owned(),
            },
        ],
        "filled 的歌词 tick 是音符的生效起点（1908 = 1920 - 12）"
    );

    let re = dir.join("Re.mid");
    let saved = dir.join("filled-imported.yeban");
    let run = invoke(&[
        "--project-sample",
        "default",
        "--import-midi",
        filled.to_str().expect("utf8"),
        "--export-midi",
        re.to_str().expect("utf8"),
        "--save-as",
        saved.to_str().expect("utf8"),
    ]);
    assert_eq!(
        run.code, 0,
        "导入 + 重导出 + 保存必须成功; stderr={}",
        run.stderr
    );
    let line = line_with(&run.stdout, "imported-midi:").expect("必须有 imported-midi 行");
    for (key, expected) in [
        ("lyrics", "4"),
        ("attached", "4"),
        ("dropped", "0"),
        ("non-utf8", "0"),
        ("empty", "0"),
    ] {
        assert_eq!(
            field(line, key).as_deref(),
            Some(expected),
            "报告字段 {key} 必须是 {expected}: {line}"
        );
    }

    // 落盘工程的原始 JSON：`"syllable"` 恰好 4 次且都是非 null（⛔ 量到原件的 4 次）。
    let json = project_json_of(&saved);
    assert_eq!(
        occurrences(&json, b"\"syllable\""),
        4,
        "导入后 project.json 里必须有 4 个 syllable 键（落点是 default 工程）"
    );
    assert_eq!(
        occurrences(&json, b"\"syllable\":\""),
        4,
        "4 个都必须是**非 null** 的音节（⛔ 只数键、⛔ 把 null 算进去）"
    );

    // 重导出：4 条 FF 05，tick/text 逐字相等（与输入文件同一组读数）。
    let re_bytes = std::fs::read(&re).expect("读重导出");
    assert_eq!(
        occurrences(&re_bytes, &[0xFF, 0x05]),
        4,
        "重导出必须仍有 4 条 FF 05（零丢失、零膨胀）"
    );
    let round_tripped = parsed(&re_bytes);
    assert_eq!(
        round_tripped.lyrics, exported.lyrics,
        "重导出的 (tick, text) 必须与导入的文件逐字相同"
    );
    assert!(
        round_tripped
            .notes
            .iter()
            .any(|note| note.start_tick == 1908),
        "1908 的微时值位置必须在重导出里保留: {:?}",
        round_tripped.notes
    );

    eprintln!(
        "[midi-round-trip/lyrics] filled={} B sha 82077185…; 导入后 syllable 键={} 非 null={}; \
         重导出 FF05={} ticks={:?}",
        bytes.len(),
        occurrences(&json, b"\"syllable\""),
        occurrences(&json, b"\"syllable\":\""),
        occurrences(&re_bytes, &[0xFF, 0x05]),
        round_tripped
            .lyrics
            .iter()
            .map(|lyric| lyric.tick)
            .collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// ⭐ `#317` **和弦正对照**（`#316` 实测的形状）：tick 960 有 **4 颗起点音符**。
///
/// - 4 条词 ⇒ 归属 **0**、`dropped=4`（⛔ 按文件序配对：文本序 `laa,re,si,sol`
///   与音高序 `64,67,72,76` 不同，一一配对会 4/4 全错）；
/// - 只有 1 条词 ⇒ 仍归属 **0**、`dropped=1`（⛔ 一颗词复制给四颗）。
///
/// 两种都断言落盘 `project.json` 里 `"syllable"` = **0** 次、重导出 `FF 05` = **0** 条
///（零错配、零膨胀）。
#[test]
fn a_chord_with_lyrics_attaches_none_and_reports_every_dropped_lyric() {
    let dir = scratch_dir("lyrics-chord");
    let cases: [(&str, [Option<&str>; 4], &str, &str); 2] = [
        (
            "four",
            [Some("laa"), Some("re"), Some("si"), Some("sol")],
            "4",
            "4",
        ),
        ("one", [Some("laa"), None, None, None], "1", "1"),
    ];
    for (tag, syllables, expected_read, expected_dropped) in cases {
        let project = lyric_chord_project(syllables);
        let container = write_container(&dir, &format!("chord-{tag}.yeban"), &project);
        let mid = dir.join(format!("chord-{tag}.mid"));
        let run = invoke(&[
            "--open",
            container.to_str().expect("utf8"),
            "--export-midi",
            mid.to_str().expect("utf8"),
        ]);
        assert_eq!(run.code, 0, "[{tag}] 导出必须成功; stderr={}", run.stderr);
        let bytes = std::fs::read(&mid).expect("读和弦 MIDI");
        let chord = parsed(&bytes);
        assert_eq!(chord.notes.len(), 4, "[{tag}] 文件里必须恰 4 颗音符");
        assert!(
            chord.notes.iter().all(|note| note.start_tick == 960),
            "[{tag}] 4 颗都在 tick 960: {:?}",
            chord.notes
        );
        assert_eq!(
            chord.notes.iter().map(|note| note.key).collect::<Vec<_>>(),
            vec![64, 67, 72, 76],
            "[{tag}] 音高序"
        );
        assert_eq!(
            chord.lyrics.len(),
            expected_read.parse::<usize>().expect("数字"),
            "[{tag}] 文件里的歌词条数"
        );
        if tag == "four" {
            assert_eq!(
                chord
                    .lyrics
                    .iter()
                    .map(|lyric| lyric.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["laa", "re", "si", "sol"],
                "4 条词的**文本序**与音高序不同 ⇒ 文件序配对必然全错"
            );
        }

        let saved = dir.join(format!("chord-{tag}-imported.yeban"));
        let re = dir.join(format!("chord-{tag}-re.mid"));
        let run = invoke(&[
            "--project-sample",
            "default",
            "--import-midi",
            mid.to_str().expect("utf8"),
            "--export-midi",
            re.to_str().expect("utf8"),
            "--save-as",
            saved.to_str().expect("utf8"),
        ]);
        assert_eq!(
            run.code, 0,
            "[{tag}] 导入 + 重导出必须成功; stderr={}",
            run.stderr
        );
        let line = line_with(&run.stdout, "imported-midi:").expect("必须有 imported-midi 行");
        for (key, expected) in [
            ("lyrics", expected_read),
            ("attached", "0"),
            ("dropped", expected_dropped),
            ("non-utf8", "0"),
            ("empty", "0"),
        ] {
            assert_eq!(
                field(line, key).as_deref(),
                Some(expected),
                "[{tag}] 报告字段 {key}: {line}"
            );
        }

        let json = project_json_of(&saved);
        assert_eq!(
            occurrences(&json, b"\"syllable\""),
            0,
            "[{tag}] 和弦不归属 ⇒ project.json 里一个 syllable 都不许有"
        );
        let re_bytes = std::fs::read(&re).expect("读重导出");
        assert_eq!(
            occurrences(&re_bytes, &[0xFF, 0x05]),
            0,
            "[{tag}] 零膨胀：重导出不许凭空多出 FF 05"
        );
        eprintln!(
            "[midi-round-trip/chord-{tag}] notes=4 tick=960 词={expected_read} ⇒ attached=0 \
             dropped={expected_dropped}；落盘 syllable=0，重导出 FF05=0"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// **判据（`#342` 改写）**：**唯一 tick 0** 的 conductor 落进工程的两个单标量。
///
/// 做法：6/8、90 BPM 的文件导入 3/4、120 BPM 的工程 ⇒ 重导出必须是 **6/8 / 90 BPM**
/// （⛔ 不再是"工程的 3/4 / 120 胜出"—— 单值有家）。这一条把"单值有家、map 没家"
/// 里的**单值**那一半变成可执行断言；map 的那一半见
/// `import_midi.rs` 的 `two_tempo_and_meter_changes_are_loud_losses_that_name_every_value`。
#[test]
fn the_import_adopts_an_unambiguous_single_conductor() {
    let dir = scratch_dir("conductor");
    // `#342` 修订：采纳 bpm/拍号要求目标工程**没有内容** ⇒ 用 content-free 目的地。
    let project = content_free_destination();
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
    assert_eq!(
        p3.tempos.len(),
        1,
        "控制臂：文件只有唯一条 tick 0 conductor（否则本判据测的不是单值）"
    );
    assert_eq!(p3.tempos[0].tick, 0);
    assert_eq!(p3.tempos[0].numerator, Some(6), "F3 的拍号分子");
    assert_eq!(p3.tempos[0].denominator_pow2, Some(3), "6/8 ⇒ dd=3");
    assert_eq!(
        p3.tempos[0].microseconds_per_quarter,
        Some(666_667),
        "90 BPM ⇒ 666667 µs"
    );

    // 导入一个**没有内容**的工程 ⇒ 重导出**采纳文件**的 conductor。
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
    let imported = line_with(&run.stdout, "imported-midi:").expect("必须有 imported-midi 行");
    assert_eq!(
        field(imported, "tempo-read").as_deref(),
        Some("1"),
        "control arm: 文件真的带一条 tempo: {imported}"
    );
    assert_eq!(
        field(imported, "tempo-dropped").as_deref(),
        Some("0"),
        "唯一 tick 0 tempo ⇒ 落地，不是损失: {imported}"
    );
    assert_eq!(
        field(imported, "time-signature-dropped").as_deref(),
        Some("0"),
        "唯一 tick 0 拍号 ⇒ 落地，不是损失: {imported}"
    );
    let bytes4 = std::fs::read(&f4).expect("读 F4");
    let p4 = parsed(&bytes4);
    assert_eq!(
        p4.tempos[0].numerator,
        Some(6),
        "落地：文件的 6/8 进 `project.time_signature`（⛔ 不再是工程的 3/4）"
    );
    assert_eq!(p4.tempos[0].denominator_pow2, Some(3), "6/8 ⇒ dd=3");
    assert_eq!(
        p4.tempos[0].microseconds_per_quarter,
        Some(666_667),
        "落地：文件的 90 BPM 进 `project.bpm`（⛔ 不再是工程的 120）"
    );
    // 而**音符**确实导进来了（conductor 的落地没有把音符吞掉）。
    assert_eq!(p4.ppq, 960);
    assert_eq!(
        p4.notes.len(),
        p3.notes.len(),
        "content-free 工程原有 0 颗 + 文件里的 {} 颗",
        p3.notes.len()
    );

    eprintln!(
        "[midi-round-trip/conductor] F3={}/{:?}/{:?} ⇒ F4={}/{:?}/{:?}（单值 conductor 落地，音符 {} 颗）",
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

/// 手工拼一份 **format 0 / 960 PPQ / 1 条 MTrk** 的 SMF（⭐ `#342` 判据夹具）。
///
/// `events` 按 tick **稳定**排序后写成 delta 序列（同一 tick 保持插入顺序 ⇒
/// `NoteOff` 排在同一个 tick 的 `NoteOn` 之前）。⛔ 不借导出器：判据要能独立于
/// "导出侧写了什么"构造（导出器只写 `FF 03` / `FF 51` / `FF 58` / `FF 05`）。
fn single_track_smf(
    track_name: Option<&str>,
    tempos: &[(u32, u32)],
    meters: &[(u32, u8, u8)],
    markers: &[(u32, &[u8])],
    notes: &[(u32, u32, u8)],
    end_tick: u32,
) -> Vec<u8> {
    fn vlq(mut value: u32) -> Vec<u8> {
        let mut out = vec![u8::try_from(value & 0x7F).expect("7 bit")];
        value >>= 7;
        while value > 0 {
            out.push(0x80 | u8::try_from(value & 0x7F).expect("7 bit"));
            value >>= 7;
        }
        out.reverse();
        out
    }
    fn meta(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, kind, u8::try_from(payload.len()).expect("短负载")];
        out.extend_from_slice(payload);
        out
    }
    let mut events: Vec<(u32, Vec<u8>)> = Vec::new();
    if let Some(name) = track_name {
        events.push((0, meta(0x03, name.as_bytes())));
    }
    for &(tick, microseconds_per_quarter) in tempos {
        events.push((
            tick,
            meta(0x51, &microseconds_per_quarter.to_be_bytes()[1..]),
        ));
    }
    for &(tick, numerator, denominator_pow2) in meters {
        events.push((tick, meta(0x58, &[numerator, denominator_pow2, 24, 8])));
    }
    for &(tick, text) in markers {
        events.push((tick, meta(0x06, text)));
    }
    for &(start, end, key) in notes {
        events.push((start, vec![0x90, key, 100]));
        events.push((end, vec![0x80, key, 0]));
    }
    events.push((end_tick, meta(0x2F, &[])));
    events.sort_by_key(|(tick, _)| *tick);

    let mut track: Vec<u8> = Vec::new();
    let mut cursor = 0_u32;
    for (tick, event) in events {
        track.extend(vlq(tick - cursor));
        cursor = tick;
        track.extend(event);
    }
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes()); // format 0
    bytes.extend_from_slice(&1u16.to_be_bytes()); // 1 条轨
    bytes.extend_from_slice(&960u16.to_be_bytes());
    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&u32::try_from(track.len()).expect("短轨").to_be_bytes());
    bytes.extend_from_slice(&track);
    bytes
}

/// ⭐ `#342` **主判据（端到端）**：一份手工拼的 SMF 导入后，`.yeban` 文档里
/// **逐项**含有：3 个段落（tick 与名字）、音轨名、bpm、拍号；且这些信息在
/// `imported-midi:` 行里**不是**损失。
///
/// 夹具：`Piano` 轨名 / 100 BPM @0 / 7/8 @0 / marker `Intro`@0、`Verse`@960、
/// `Sec "1"\n—`@1920（**引号 + 换行 + 破折号**）/ 4 颗音符 / EOT@2880。
/// ⇒ 段落 = `[0,960) [960,1920) [1920,2880)`（最后一个延伸到文件结束）。
#[test]
fn a_marker_track_lands_sections_track_name_bpm_and_meter_in_the_document() {
    let dir = scratch_dir("metadata-round-trip");
    // 目的地：**没有内容**（否则 bpm/拍号不会被采纳），音轨名先设成别的值。
    let project = content_free_destination();
    let destination_track = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("有一条非 master 音轨");
    let container = write_container(&dir, "dest.yeban", &project);
    let container_arg = container.to_str().expect("utf8");

    let marker_text = "Sec \"1\"\n—".as_bytes();
    let bytes = single_track_smf(
        Some("Piano"),
        &[(0, 600_000)], // 100 BPM
        &[(0, 7, 3)],    // 7/8
        &[(0, b"Intro"), (960, b"Verse"), (1920, marker_text)],
        &[
            (0, 480, 60),
            (480, 960, 64),
            (960, 1440, 67),
            (1440, 1920, 72),
        ],
        2880,
    );
    let mid = dir.join("metadata.mid");
    std::fs::write(&mid, &bytes).expect("写夹具");
    let saved = dir.join("after.yeban");

    let run = invoke(&[
        "--open",
        container_arg,
        "--import-midi",
        mid.to_str().expect("utf8"),
        "--save-as",
        saved.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "导入 + 保存必须成功; stderr={}", run.stderr);
    let line = line_with(&run.stdout, "imported-midi:").expect("必须有 imported-midi 行");
    for (key, want) in [
        ("marker-read", "3"),
        ("marker-dropped", "0"),
        ("track-name-read", "1"),
        ("track-name-dropped", "0"),
        ("tempo-read", "1"),
        ("tempo-dropped", "0"),
        ("time-signature-read", "1"),
        ("time-signature-dropped", "0"),
    ] {
        assert_eq!(
            field(line, key).as_deref(),
            Some(want),
            "报告 {key}: {line}"
        );
    }

    let opened = open_project_file(&saved).expect("回读保存的工程");
    // ---- 段落：tick 与名字逐项 ----
    let mut sections: Vec<(String, u64, u64)> = opened
        .sections
        .values()
        .map(|section| (section.name.clone(), section.start_tick, section.end_tick))
        .collect();
    sections.sort_by_key(|(_, start, _)| *start);
    assert_eq!(
        sections,
        vec![
            ("Intro".to_owned(), 0, 960),
            ("Verse".to_owned(), 960, 1920),
            // ⭐ 名字逐字节保留（引号 / 换行 / 破折号都不在模型层转义）。
            ("Sec \"1\"\n—".to_owned(), 1920, 2880),
        ],
        "marker → SectionV3：区间到下一个 marker / end_tick"
    );
    for pair in sections.windows(2) {
        assert!(
            pair[0].2 <= pair[1].1,
            "导入的段落两两不重叠: {:?} 与 {:?}",
            pair[0],
            pair[1]
        );
    }
    assert_eq!(
        opened
            .sections
            .values()
            .filter(|section| section.end_tick <= section.start_tick)
            .count(),
        0,
        "⛔ 不许有 end <= start 的段落（模型会拒绝整份工程）"
    );

    // ---- 音轨名 / bpm / 拍号 ----
    assert_eq!(
        opened.tracks[&destination_track].name, "Piano",
        "唯一一个非空轨名 ⇒ 落进目标音轨的 TrackV3::name"
    );
    assert_eq!(opened.bpm, 100.0, "唯一 tick 0 tempo ⇒ project.bpm");
    assert_eq!(
        opened.time_signature,
        TimeSignature {
            numerator: 7,
            denominator: 8
        },
        "唯一 tick 0 拍号 ⇒ project.time_signature"
    );
    assert_eq!(opened.validate(), Ok(()), "落盘后的工程必须整体合法");

    eprintln!(
        "[#342/round-trip] 段落={:?}；轨名={}；bpm={}；拍号={}/{}；line={}",
        sections,
        opened.tracks[&destination_track].name,
        opened.bpm,
        opened.time_signature.numerator,
        opened.time_signature.denominator,
        line
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的地工程（`#342` 判据共用）：清空演示工程的段落、120 BPM、4/4，
/// 目标音轨名先设成 `Original`（这样"轨名被采纳"才可观测）。
fn metadata_destination() -> YebanProjectV1 {
    let mut project = demo_project();
    project.sections.clear();
    project.bpm = 120.0;
    project.time_signature = TimeSignature {
        numerator: 4,
        denominator: 4,
    };
    let destination_track = *project
        .tracks
        .keys()
        .find(|id| **id != project.master_bus_track_id)
        .expect("有一条非 master 音轨");
    project
        .tracks
        .get_mut(&destination_track)
        .expect("目标音轨")
        .name = "Original".to_owned();
    project
}

/// 目的地工程（`#342` **采纳臂**）：在 [`metadata_destination`] 之上**去掉全部内容**
/// （音符 / 音频片段 / 自动化点；段落已被它清空），但保留**空 MIDI 片段 + 摆放**
/// —— 导入必须有落点。判定与 `import_midi::import_target_is_empty` 同一口径。
fn content_free_destination() -> YebanProjectV1 {
    let mut project = metadata_destination();
    for entry in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut entry.content {
            notes.clear();
        }
    }
    let audio_clips: Vec<EntityId> = project
        .clip_pool
        .iter()
        .filter(|(_, entry)| matches!(&entry.content, ClipContent::Audio { .. }))
        .map(|(id, _)| *id)
        .collect();
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| !audio_clips.contains(&placement.clip_id));
        for lane in track.automation_lanes.values_mut() {
            lane.points.clear();
        }
    }
    project.clip_pool.retain(|id, _| !audio_clips.contains(id));
    project.validate().expect("content-free 目的地必须合法");
    project
}

/// 导入一份夹具并回读保存的工程（`--open <dest> --import-midi <mid> --save-as`）。
fn import_and_reopen(
    tag: &str,
    destination: &YebanProjectV1,
    bytes: &[u8],
) -> (YebanProjectV1, String) {
    let dir = scratch_dir(tag);
    let container = write_container(&dir, "dest.yeban", destination);
    let mid = dir.join("fixture.mid");
    std::fs::write(&mid, bytes).expect("写夹具");
    let saved = dir.join("after.yeban");
    let run = invoke(&[
        "--open",
        container.to_str().expect("utf8"),
        "--import-midi",
        mid.to_str().expect("utf8"),
        "--save-as",
        saved.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "导入 + 保存必须成功; stderr={}", run.stderr);
    let line = line_with(&run.stdout, "imported-midi:")
        .expect("必须有 imported-midi 行")
        .to_owned();
    let opened = open_project_file(&saved).expect("回读保存的工程");
    let _ = std::fs::remove_dir_all(&dir);
    (opened, line)
}

/// ⭐ `#342`：**两条 tempo** 的 map 没有家 ⇒ `project.bpm` **原样**（120），
/// `imported-midi:` 行**点名两个值**（⛔ 不许静默挑一个）。
#[test]
fn two_tempo_changes_are_a_loud_loss_and_the_project_bpm_is_not_silently_replaced() {
    let destination = metadata_destination();
    // 100 BPM @0 → 150 BPM @960；两个值都与目的地工程的 120 不同。
    let bytes = single_track_smf(
        None,
        &[(0, 600_000), (960, 400_000)],
        &[(0, 4, 2)],
        &[],
        &[(0, 480, 60), (480, 960, 64)],
        960,
    );
    let (opened, line) = import_and_reopen("two-tempos", &destination, &bytes);
    let line = line.as_str();
    assert_eq!(
        opened.bpm, 120.0,
        "tempo map 没有家 ⇒ ⛔ 不许把 100 或 150 写进单标量 `project.bpm`"
    );
    assert_eq!(field(line, "tempo-read").as_deref(), Some("2"), "{line}");
    assert_eq!(field(line, "tempo-dropped").as_deref(), Some("2"), "{line}");
    assert!(
        line.contains("100.00 BPM @0") && line.contains("150.00 BPM @960"),
        "损失条目必须**点名两个值与 tick**: {line}"
    );
    assert!(
        line.contains("tempo-reason="),
        "必须给出精确原因（MCP 可逐字引用）: {line}"
    );
}

/// ⭐ `#342`：**两条拍号**（5/8 → 3/4）的 map 没有家 ⇒ `project.time_signature`
/// **原样**（4/4），损失条目**点名两个值**（⛔ 不许静默挑一个）。
#[test]
fn two_meter_changes_are_a_loud_loss_and_the_project_meter_is_not_silently_replaced() {
    let destination = metadata_destination();
    let bytes = single_track_smf(
        None,
        &[(0, 500_000)],           // 120 BPM（只有一条 tempo ⇒ 会落地）
        &[(0, 5, 3), (960, 3, 2)], // 5/8 → 3/4
        &[],
        &[(0, 480, 60), (480, 960, 64)],
        960,
    );
    let (opened, line) = import_and_reopen("two-meters", &destination, &bytes);
    let line = line.as_str();
    assert_eq!(
        opened.time_signature,
        TimeSignature {
            numerator: 4,
            denominator: 4
        },
        "meter map 没有家 ⇒ ⛔ 不许把 5/8 或 3/4 写进单标量 `project.time_signature`"
    );
    assert_eq!(
        field(line, "time-signature-read").as_deref(),
        Some("2"),
        "{line}"
    );
    assert_eq!(
        field(line, "time-signature-dropped").as_deref(),
        Some("2"),
        "{line}"
    );
    assert!(
        line.contains("5/8 @0") && line.contains("3/4 @960"),
        "损失条目必须**点名两个值与 tick**: {line}"
    );
}

/// ⭐ `#342` 修订：**非空工程**的 `bpm` / `time_signature` 必须**逐位不变**，
/// 文件里的单值 tempo / 拍号作为响亮损失点名（含值与 tick），并说明工程已有内容。
#[test]
fn a_non_empty_project_keeps_bpm_and_meter_byte_identical_and_names_the_values() {
    let destination = metadata_destination();
    assert!(
        !yeban_app::import_midi::import_target_is_empty(&destination),
        "metadata_destination 必须是有内容的工程（否则本判据测的是采纳臂）"
    );
    // 100 BPM @0、7/8 @0 —— 与目的地的 120 / 4-4 都不同。
    let bytes = single_track_smf(
        None,
        &[(0, 600_000)],
        &[(0, 7, 3)],
        &[],
        &[(0, 480, 60)],
        960,
    );
    let (opened, line) = import_and_reopen("non-empty-globals", &destination, &bytes);
    let line = line.as_str();

    // ⭐ **逐位**不变（f64 用 `to_bits`，不是 `==`）。
    assert_eq!(
        opened.bpm.to_bits(),
        destination.bpm.to_bits(),
        "非空工程的 `project.bpm` 必须逐位不变（{} vs {}）",
        opened.bpm,
        destination.bpm
    );
    assert_eq!(
        opened.time_signature, destination.time_signature,
        "非空工程的 `project.time_signature` 必须一字不变"
    );

    assert_eq!(field(line, "tempo-read").as_deref(), Some("1"), "{line}");
    assert_eq!(field(line, "tempo-dropped").as_deref(), Some("1"), "{line}");
    assert!(
        line.contains("100.00 BPM @0") && line.contains("already has content"),
        "tempo 损失必须点名值与 tick，并说明工程已有内容: {line}"
    );
    assert!(
        line.contains("time-signature-dropped=1"),
        "文件里的 7/8 必须报成损失: {line}"
    );
    assert!(
        line.contains("7/8 @0") && line.contains("time-signature-not-adopted-non-empty-project"),
        "拍号损失必须点名值与 tick: {line}"
    );
}

/// ⭐ `#342`：同 tick 顶替 + 末尾零长度必须**响亮**报丢、⛔ 不许变成段落；
/// 原因里的引号在回执里被换成 `'`（回执是引号定界，不认转义）。
#[test]
fn zero_length_and_duplicate_markers_are_reported_and_never_become_sections() {
    let destination = metadata_destination();
    // A@0 被同 tick 的 B 顶掉；C@960 与 end_tick=960 重合 ⇒ 零长度。
    let bytes = single_track_smf(
        None,
        &[(0, 500_000)],
        &[(0, 4, 2)],
        &[(0, b"A"), (0, b"B"), (960, b"bad\"name")],
        &[(0, 480, 60)],
        960,
    );
    let (opened, line) = import_and_reopen("zero-length", &destination, &bytes);
    let line = line.as_str();
    let mut sections: Vec<(String, u64, u64)> = opened
        .sections
        .values()
        .map(|section| (section.name.clone(), section.start_tick, section.end_tick))
        .collect();
    sections.sort_by_key(|(_, start, _)| *start);
    assert_eq!(
        sections,
        vec![("B".to_owned(), 0, 960)],
        "同 tick 后一条胜出；零长度的 C 不生成段落"
    );
    assert_eq!(field(line, "marker-read").as_deref(), Some("3"), "{line}");
    assert_eq!(
        field(line, "marker-dropped").as_deref(),
        Some("2"),
        "{line}"
    );
    assert!(
        line.contains("superseded") && line.contains("`A`"),
        "同 tick 顶替必须点名被顶掉的 A: {line}"
    );
    assert!(
        line.contains("zero-length") && line.contains("bad'name"),
        "零长度必须点名 C，且回执里的引号已被换成 ' : {line}"
    );
    assert!(
        !line.contains("bad\"name"),
        "回执不许带着原始引号（会把 loss_detail 截断）: {line}"
    );
}

/// ⭐ `#342`：没有 marker 的文件**不新增**段落（也**不**编一条丢失原因）。
#[test]
fn a_file_without_markers_adds_no_sections() {
    let destination = metadata_destination();
    let bytes = single_track_smf(
        None,
        &[(0, 500_000)],
        &[(0, 4, 2)],
        &[],
        &[(0, 480, 60), (480, 960, 64)],
        960,
    );
    let (opened, line) = import_and_reopen("no-markers", &destination, &bytes);
    let line = line.as_str();
    assert!(opened.sections.is_empty(), "没有 marker ⇒ 一个段落都不新增");
    // `aux_fields` 的口径：`read == 0` 的类别**不打印** ⇒ 不是 `marker-read=0`，而是缺席。
    assert_eq!(field(line, "marker-read").as_deref(), None, "{line}");
    assert!(
        !line.contains("marker-read=")
            && !line.contains("marker-dropped=")
            && !line.contains("marker-reason="),
        "read=0 的类别不许出现，更不许编一条原因: {line}"
    );
}

/// 手工拼一份**含全部 24 类 aux 事件**的 SMF：format 1、3 轨、960 PPQ、
/// 5 音符、2 歌词（1 唯一归属 + 1 因跨轨同 tick 双起点而不归属）。
///
/// ⛔ 借导出器构造：导出器只写 `FF 03` / `FF 51` / `FF 58` / `FF 05` 四类，
/// 用它会让"没写"与"没读"混在一起（本判据要的是后者）。
fn all_aux_events_smf() -> Vec<u8> {
    fn vlq(mut value: u32) -> Vec<u8> {
        let mut out = vec![u8::try_from(value & 0x7F).expect("7 bit")];
        value >>= 7;
        while value > 0 {
            out.push(0x80 | u8::try_from(value & 0x7F).expect("7 bit"));
            value >>= 7;
        }
        out.reverse();
        out
    }
    fn meta(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, kind, u8::try_from(payload.len()).expect("短负载")];
        out.extend_from_slice(payload);
        out
    }
    fn track(events: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (delta, event) in events {
            body.extend(vlq(*delta));
            body.extend_from_slice(event);
        }
        let mut out = b"MTrk".to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).expect("短轨").to_be_bytes());
        out.extend(body);
        out
    }

    let conductor = track(&[
        (0, meta(0x00, &[0x00, 0x01])),          // TrackNumber
        (0, meta(0x03, b"Conductor")),           // TrackName
        (0, meta(0x51, &[0x03, 0xD0, 0x90])),    // Tempo 250000（240 BPM）
        (0, meta(0x58, &[6, 3, 24, 8])),         // TimeSignature 6/8
        (0, meta(0x59, &[2, 0])),                // KeySignature 2♯ major
        (0, meta(0x06, b"Verse")),               // Marker
        (0, meta(0x05, b"la")),                  // Lyric（tick 0 有 2 颗起点音符 ⇒ 不归属）
        (0, meta(0x01, b"a text meta")),         // Text
        (0, meta(0x02, b"(c) 2026")),            // Copyright
        (0, meta(0x04, b"Piano")),               // InstrumentName
        (0, meta(0x07, b"Cue")),                 // CuePoint
        (0, meta(0x08, b"P1")),                  // ProgramName
        (0, meta(0x09, b"Dev")),                 // DeviceName
        (0, meta(0x20, &[1])),                   // MidiChannelPrefix
        (0, meta(0x21, &[0])),                   // MidiPort
        (0, meta(0x54, &[0; 5])),                // SmpteOffset
        (0, meta(0x7F, &[1, 2, 3])),             // SequencerSpecific
        (0, meta(0x60, &[0xAB])),                // Unknown meta id
        (0, vec![0xF0, 0x03, 0x01, 0x02, 0xF7]), // SysEx
        (0, vec![0xF7, 0x02, 0x03, 0x04]),       // Escape
        (480, vec![0xE0, 0x00, 0x40]),           // PitchBend 8192
        (0, vec![0xB0, 7, 100]),                 // ControlChange
        (0, vec![0xD0, 40]),                     // ChannelAftertouch
        (0, vec![0xA0, 60, 33]),                 // PolyAftertouch
        (0, vec![0xC0, 33]),                     // ProgramChange
        (480, meta(0x51, &[0x07, 0xA1, 0x20])),  // Tempo 500000（120 BPM）
        (1440, meta(0x06, b"Chorus")),           // Marker（第 2 条）
        (0, meta(0x58, &[3, 2, 24, 8])),         // TimeSignature 3/4（第 2 条）
        (0, meta(0x2F, &[])),                    // EndOfTrack
    ]);
    let lead = track(&[
        (0, meta(0x03, b"Lead")), // TrackName
        (0, vec![0x90, 60, 100]),
        (480, vec![0x80, 60, 64]),
        (0, vec![0x90, 64, 90]),
        (0, meta(0x05, b"re")), // tick 480 恰 1 颗起点音符 ⇒ 唯一归属
        (480, vec![0x80, 64, 32]),
        (0, vec![0x90, 67, 80]),
        (480, vec![0x80, 67, 16]),
        (0, meta(0x2F, &[])),
    ]);
    let bass = track(&[
        (0, meta(0x03, b"Bass")), // TrackName
        (0, vec![0x91, 48, 110]),
        (960, vec![0x81, 48, 8]),
        (0, vec![0x91, 52, 100]),
        (960, vec![0x81, 52, 4]),
        (0, meta(0x2F, &[])),
    ]);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes()); // format 1
    bytes.extend_from_slice(&3u16.to_be_bytes()); // 3 tracks
    bytes.extend_from_slice(&960u16.to_be_bytes());
    bytes.extend_from_slice(&conductor);
    bytes.extend_from_slice(&lead);
    bytes.extend_from_slice(&bass);
    bytes
}

/// ⭐ `#318` **端到端主判据**：导入一份含**全部 24 类 aux 事件**的文件 ⇒
/// `imported-midi:` 行逐类说出 `read` / `dropped`，而重导出的文件**不再有**那些事件。
///
/// 判据分三层：
/// 1. **控制臂**：先按字节独立数一遍**源文件**（⛔ 读解析器），证明夹具真的有那些事件；
/// 2. **报告**：逐类 `read == 期望`、`dropped == 期望`（⭐ `#342`：四类可能部分落地）；
/// 3. **后果**：重导出里 `FF 59` / `FF 06` / CC / 弯音 / 触后 / PC / SysEx / Escape
///    逐条为 0；`tempo` / 拍号是**工程的** 120 BPM / 4/4；`MTrk` 从 3 变 1 条音符轨；
///    音符通道从 {0,1} 变 {0}。⛔ 只报数不看后果 = 读数可能量的是别的东西。
#[test]
fn the_import_reports_every_aux_event_kind_as_read_and_dropped() {
    let dir = scratch_dir("aux-readout");
    let bytes = all_aux_events_smf();
    let mid = dir.join("all-aux.mid");
    std::fs::write(&mid, &bytes).expect("写夹具");

    // ── ① 控制臂：独立按字节数源文件（单位 = 子序列出现次数） ──
    for (needle, expected, label) in [
        (vec![0xFF, 0x00], 1, "FF 00 track number"),
        (vec![0xFF, 0x01], 1, "FF 01 text"),
        (vec![0xFF, 0x02], 1, "FF 02 copyright"),
        (vec![0xFF, 0x03], 3, "FF 03 track name"),
        (vec![0xFF, 0x04], 1, "FF 04 instrument name"),
        (vec![0xFF, 0x05], 2, "FF 05 lyric"),
        (vec![0xFF, 0x06], 2, "FF 06 marker"),
        (vec![0xFF, 0x07], 1, "FF 07 cue point"),
        (vec![0xFF, 0x08], 1, "FF 08 program name"),
        (vec![0xFF, 0x09], 1, "FF 09 device name"),
        (vec![0xFF, 0x20], 1, "FF 20 midi channel prefix"),
        (vec![0xFF, 0x21], 1, "FF 21 midi port"),
        (vec![0xFF, 0x51], 2, "FF 51 tempo"),
        (vec![0xFF, 0x54], 1, "FF 54 smpte offset"),
        (vec![0xFF, 0x58], 2, "FF 58 time signature"),
        (vec![0xFF, 0x59], 1, "FF 59 key signature"),
        (vec![0xFF, 0x60], 1, "FF 60 unknown meta"),
        (vec![0xFF, 0x7F], 1, "FF 7F sequencer specific"),
        (vec![0xB0, 7, 100], 1, "CC"),
        (vec![0xC0, 33], 1, "program change"),
        (vec![0xD0, 40], 1, "channel aftertouch"),
        (vec![0xA0, 60, 33], 1, "poly aftertouch"),
        (vec![0xE0, 0x00, 0x40], 1, "pitch bend"),
        (vec![0xF0, 0x03, 0x01, 0x02, 0xF7], 1, "SysEx"),
        (vec![0xF7, 0x02, 0x03, 0x04], 1, "escape"),
    ] {
        assert_eq!(
            occurrences(&bytes, &needle),
            expected,
            "控制臂：源文件里 {label} 必须恰 {expected} 次（⛔ 不靠解析器）"
        );
    }

    // ── ② 导入 + 重导出 ──
    let out = dir.join("out.mid");
    let saved = dir.join("out.yeban");
    let run = invoke(&[
        "--project-sample",
        "default",
        "--import-midi",
        mid.to_str().expect("utf8"),
        "--export-midi",
        out.to_str().expect("utf8"),
        "--save-as",
        saved.to_str().expect("utf8"),
    ]);
    assert_eq!(run.code, 0, "导入 + 重导出必须成功; stderr={}", run.stderr);
    let line = line_with(&run.stdout, "imported-midi:").expect("必须有 imported-midi 行");
    for (key, expected) in [
        ("notes", "5"),
        ("lyrics", "2"),
        ("attached", "1"),
        ("dropped", "1"),
        ("non-utf8", "0"),
        ("empty", "0"),
        ("format", "1"),
        ("file-tracks", "3"),
        ("track-partitions-dropped", "0"),
        ("channels", "2"),
        ("note-channel-read", "5"),
        ("note-channel-dropped", "0"),
        ("note-off-velocity-read", "5"),
        ("note-off-velocity-dropped", "5"),
    ] {
        assert_eq!(
            field(line, key).as_deref(),
            Some(expected),
            "报告字段 {key}: {line}"
        );
    }

    // ── ③ 逐类 read/dropped（24 类，一个不缺；⭐ `#342`：四类可能部分落地） ──
    //
    // 本夹具是 `#342` 之前的形状：2 tempo（240@0 → 120@960）、2 拍号（6/8@0 →
    // 3/4@2400）、3 个不同轨名、2 marker（`Verse`@0 → `Chorus`@2400，后者与
    // `end_tick=2400` 重合 ⇒ 零长度）。⇒ 四类里只有 **1 条 marker** 落地。
    let expected: [(MidiAuxEventKind, usize, usize); MidiAuxEventKind::COUNT] = [
        (MidiAuxEventKind::Tempo, 2, 2),
        (MidiAuxEventKind::TimeSignature, 2, 2),
        (MidiAuxEventKind::TrackName, 3, 3),
        (MidiAuxEventKind::Marker, 2, 1),
        (MidiAuxEventKind::KeySignature, 1, 1),
        (MidiAuxEventKind::ControlChange, 1, 0),
        (MidiAuxEventKind::ProgramChange, 1, 1),
        (MidiAuxEventKind::PitchBend, 1, 1),
        (MidiAuxEventKind::PolyAftertouch, 1, 1),
        (MidiAuxEventKind::ChannelAftertouch, 1, 1),
        (MidiAuxEventKind::SysEx, 1, 1),
        (MidiAuxEventKind::Escape, 1, 1),
        (MidiAuxEventKind::Text, 1, 1),
        (MidiAuxEventKind::Copyright, 1, 1),
        (MidiAuxEventKind::InstrumentName, 1, 1),
        (MidiAuxEventKind::TrackNumber, 1, 1),
        (MidiAuxEventKind::CuePoint, 1, 1),
        (MidiAuxEventKind::ProgramName, 1, 1),
        (MidiAuxEventKind::DeviceName, 1, 1),
        (MidiAuxEventKind::MidiChannelPrefix, 1, 1),
        (MidiAuxEventKind::MidiPort, 1, 1),
        (MidiAuxEventKind::SmpteOffset, 1, 1),
        (MidiAuxEventKind::SequencerSpecific, 1, 1),
        (MidiAuxEventKind::UnknownMeta, 1, 1),
    ];
    let mut stored_kinds = 0_usize;
    for (kind, read_want, dropped_want) in expected {
        let key = kind.key();
        let read_key = format!("{key}-read");
        let dropped_key = format!("{key}-dropped");
        assert_eq!(
            field(line, &read_key).as_deref(),
            Some(read_want.to_string().as_str()),
            "{} 的 read: {line}",
            kind.key()
        );
        assert_eq!(
            field(line, &dropped_key).as_deref(),
            Some(dropped_want.to_string().as_str()),
            "{} 的 dropped: {line}",
            kind.key()
        );
        if dropped_want < read_want {
            stored_kinds += 1;
            if dropped_want > 0 {
                assert!(
                    line.contains(&format!("{key}-reason=")),
                    "有损失但没落地全量的类别必须带原因（{key}）: {line}"
                );
            }
        }
        if dropped_want > 0 {
            assert!(
                line.contains(&format!("{key}-reason=")),
                "dropped>0 的类别必须带精确原因（{key}）: {line}"
            );
        }
    }
    assert_eq!(
        stored_kinds, 2,
        "本夹具里 marker 部分落地（1/2），CC 7 消费落地（1/1）"
    );

    // ── ④ 后果：重导出不再带那些事件，conductor / 轨分区 / 通道被工程的取代 ──
    let out_bytes = std::fs::read(&out).expect("读重导出");
    for (needle, label) in [
        (vec![0xFF, 0x00], "track number"),
        (vec![0xFF, 0x06], "marker"),
        (vec![0xFF, 0x59], "key signature"),
        (vec![0xFF, 0x01], "text"),
        (vec![0xFF, 0x02], "copyright"),
        (vec![0xFF, 0x04], "instrument name"),
        (vec![0xFF, 0x07], "cue point"),
        (vec![0xFF, 0x08], "program name"),
        (vec![0xFF, 0x09], "device name"),
        (vec![0xFF, 0x20], "midi channel prefix"),
        (vec![0xFF, 0x21], "midi port"),
        (vec![0xFF, 0x54], "smpte offset"),
        (vec![0xFF, 0x60], "unknown meta"),
        (vec![0xFF, 0x7F], "sequencer specific"),
        (vec![0xC0, 33], "program change"),
        (vec![0xD0, 40], "channel aftertouch"),
        (vec![0xA0, 60, 33], "poly aftertouch"),
        (vec![0xE0, 0x00, 0x40], "pitch bend"),
        (vec![0xF0, 0x03, 0x01, 0x02, 0xF7], "SysEx"),
        (vec![0xF7, 0x02, 0x03, 0x04], "escape"),
    ] {
        assert_eq!(
            occurrences(&out_bytes, &needle),
            0,
            "重导出不许再有 {label}（源文件里有，导入没消费）"
        );
    }
    assert_eq!(
        occurrences(&out_bytes, &[0xB0, 7, 100]),
        1,
        "CC 7 经音量泳道消费并如实重导出"
    );
    assert_eq!(
        occurrences(&out_bytes, &[0xFF, 0x51]),
        1,
        "只有工程的一条 tempo"
    );
    assert_eq!(
        occurrences(&out_bytes, &[0xFF, 0x58]),
        1,
        "只有工程的一条拍号"
    );
    assert_eq!(
        occurrences(&out_bytes, &[0xFF, 0x05]),
        1,
        "只保留唯一归属的那条词"
    );
    assert_eq!(
        occurrences(&out_bytes, &[0xFF, 0x03]),
        3,
        "conductor + 两条工程音符轨名（多轨拆分创建新轨）"
    );

    let exported = parsed(&out_bytes);
    assert_eq!(exported.tracks, 3, "conductor + 2 条音符轨（多轨拆分）");
    assert_eq!(exported.notes.len(), 11, "工程原有 6 颗 + 文件里的 5 颗");
    assert_eq!(
        exported.tempos[0].microseconds_per_quarter,
        Some(500_000),
        "120 BPM = 工程的 bpm（文件的 250000/500000 都被忽略）"
    );
    assert_eq!(
        (
            exported.tempos[0].numerator,
            exported.tempos[0].denominator_pow2
        ),
        (Some(4), Some(2)),
        "4/4 = 工程的拍号（文件的 6/8、3/4 都被忽略）"
    );
    let exported_channels: std::collections::BTreeSet<u8> =
        exported.notes.iter().map(|note| note.channel).collect();
    assert_eq!(
        exported_channels,
        std::collections::BTreeSet::from([0, 1]),
        "通道 {{0, 1}} 完整保留在音符上并重导出"
    );
    assert_eq!(exported.lyrics.len(), 1, "只保留 tick 480 的唯一归属词");
    assert_eq!(exported.lyrics[0].text, "re");
    assert_eq!(
        exported.aux_events.count(MidiAuxEventKind::Tempo),
        1,
        "重导出只剩工程 conductor 的一条 tempo"
    );
    assert_eq!(
        exported.aux_events.count(MidiAuxEventKind::TimeSignature),
        1
    );
    assert_eq!(exported.aux_events.count(MidiAuxEventKind::TrackName), 3);
    assert_eq!(
        exported.aux_events.count(MidiAuxEventKind::ControlChange),
        1,
        "重导出带回 1 条 CC 7"
    );
    assert_eq!(
        exported.aux_events.total(),
        6,
        "重导出的 aux 事件包含 tempo 1 + 拍号 1 + 轨名 3 + CC 1"
    );
    assert_eq!(exported.control_changes.len(), 1);
    assert_eq!(exported.control_changes[0].controller, 7);
    assert_eq!(exported.control_changes[0].value, 100);

    eprintln!(
        "[midi-round-trip/aux-readout] 源 316 B：24 类 aux（29 条）+ 通道 {{0,1}} + 3 轨 ⇒ \
         报告逐类 read/dropped；重导出弃掉全部源 aux、拆分成 {} 条 MTrk、通道 {{0,1}}",
        exported.tracks
    );

    let _ = std::fs::remove_dir_all(&dir);
}
