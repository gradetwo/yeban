//! MusicXML 只读导入的**本机探针**（不是判据：判据在 `tests/musicxml_contract.rs`）。
//!
//! 用法：
//!
//! ```text
//! cargo run -p yeban-midi --example musicxml_probe -- /tmp/musicxml
//! cargo run -p yeban-midi --example musicxml_probe -- a.musicxml b.mxl
//! ```
//!
//! 每个路径可以是文件或目录；目录会被**排序**后逐个处理（顺序确定）。
//!
//! 打印的读数（单位写明）：
//!
//! - `bytes`：文件字节数（`stat` 的 `st_size`，不是磁盘占用）；
//! - `divisions`：文件声明的每四分音符单位数（缺省 1）；
//! - `parts`：`id/名称/音符数`；
//! - `notes`：音符**条目数**（不是行数）；tie 合并后的音符；
//! - `ticks`：全部音符的最小起始 tick 与最大结束 tick（单位：**tick**，960 PPQ）；
//! - `digest`：FNV-1a 64 位摘要，输入是**按输出顺序**的 `(key, start_tick, duration_ticks)`
//!   —— 用来和另一份独立实现（本票的 Python 参考模型）对比；
//! - `ignored` / `unsupported`：登记的**元素名条目数**。
//!
//! 另外对每个文件的字节做**截断 / 翻转 / 插入**探针，最后一行报告
//! `fuzz runs=<n> panics=0`。⚠️ 这是探针，不是穷尽证明：它只说明"这些变形没 panic"。

use std::path::{Path, PathBuf};

use yeban_midi::musicxml::{MusicXmlScore, parse_musicxml};

/// FNV-1a 64：与 Python 参考模型用**同一常数**，便于逐文件对账。
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn digest(score: &MusicXmlScore) -> u64 {
    let mut hash = FNV_OFFSET;
    for note in score.parts.iter().flat_map(|part| part.notes.iter()) {
        for value in [u64::from(note.key), note.start_tick, note.duration_ticks] {
            hash ^= value;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("用法: musicxml_probe <文件或目录>...");
        std::process::exit(2);
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for argument in &args {
        let path = PathBuf::from(argument);
        if path.is_dir() {
            let mut children: Vec<PathBuf> = std::fs::read_dir(&path)
                .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
                .unwrap_or_default();
            children.sort();
            paths.extend(children.into_iter().filter(|child| child.is_file()));
        } else {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();

    let mut fuzz_runs: usize = 0;
    for path in &paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("{}: READ ERROR {error}", label(path));
                continue;
            }
        };
        match parse_musicxml(&bytes) {
            Ok(score) => {
                let parts: Vec<String> = score
                    .parts
                    .iter()
                    .map(|part| format!("{}/{}/{}", part.id, part.name, part.notes.len()))
                    .collect();
                let (min_tick, max_tick) = match score.tick_range() {
                    Some((low, high)) => (low.to_string(), high.to_string()),
                    None => ("-".to_owned(), "-".to_owned()),
                };
                println!(
                    "{}: OK bytes={} divisions={} notes={} ticks={}..{} digest={:016x} \
                     ignored_names={} unsupported={:?} parts=[{}]",
                    label(path),
                    bytes.len(),
                    score.divisions,
                    score.note_count(),
                    min_tick,
                    max_tick,
                    digest(&score),
                    score.ignored_elements.len(),
                    score.unsupported_elements,
                    parts.join(", ")
                );
            }
            Err(error) => {
                println!("{}: ERR bytes={} {error}", label(path), bytes.len());
            }
        }
        fuzz_runs += fuzz(path, &bytes);
    }
    println!("fuzz runs={fuzz_runs} panics=0");
}

fn label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 对一份字节做确定性的变形（截断 / 翻转 / 插入），每次变形跑一遍 parser。
///
/// 返回跑的次数。任何 panic 都会终止进程 —— 因此"正常跑完"就是"零 panic"的证据。
fn fuzz(path: &Path, bytes: &[u8]) -> usize {
    let mut runs = 0usize;
    let label = label(path);
    for cut in (0..bytes.len()).step_by(9973) {
        let _ = parse_musicxml(&bytes[..cut]);
        runs += 1;
    }
    let mut copy = bytes.to_vec();
    for index in (0..bytes.len()).step_by(9973) {
        for replacement in [0x00u8, b'&', b'<', 0xff] {
            copy[index] = replacement;
            let _ = parse_musicxml(&copy);
            runs += 1;
        }
        copy[index] = bytes[index];
    }
    let mut inserted = bytes.to_vec();
    for index in (0..bytes.len()).step_by(19997) {
        inserted.insert(index, b'<');
        let _ = parse_musicxml(&inserted);
        runs += 1;
        inserted.remove(index);
    }
    println!("  (fuzz {label}: {runs} runs, 0 panics)");
    runs
}
