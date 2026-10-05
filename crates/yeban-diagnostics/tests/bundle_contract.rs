//! `[D56]` 诊断包的**契约判据**（判据 1 / 2 / 3 / 5）。
//!
//! 为什么要有它: 采集器与 MCP 工具都已接线, 但"接上了"不等于"包是对的"。本文件把 D56 的四条判据
//! 变成可复跑断言, 每条都能失败。
//!
//! - 判据 1（必需条目在场）：`MANIFEST.txt` / `env.txt` / `git.txt` 必须在; 调用方给了状态与配置时它们也要在。
//! - 判据 2（逐项 sha256）：读回每个条目, 重算 sha256, 与 `MANIFEST.txt` 里的记录**逐个比对**。
//! - 判据 3（脱敏）：扫描包内**所有**文件, 断言不出现真实的 `$HOME` 路径。
//! - 判据 5（牙测）：把 `MANIFEST.txt` 从包内清单里"拿走"的等价情形 —— 我们用"条目集合与 MANIFEST 记录必须一一对应"
//!   来体现; 并在下面单独断言"少一项就必须被发现"。
use std::collections::BTreeMap;
use std::io::Read as _;

use yeban_diagnostics::{BundleInputs, export_diagnostics};

/// 造一个隔离的输出目录（测试专用），并保证测试结束清理。
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-diag-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("建 scratch 目录");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 读回包内所有条目：名字 → 字节。
fn read_entries(zip_path: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    let file = std::fs::File::open(zip_path).expect("打开 zip");
    let mut archive = zip::ZipArchive::new(file).expect("解析 zip");
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("取条目");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("读条目");
        out.insert(name, bytes);
    }
    out
}

/// 从 `MANIFEST.txt` 解析出 `名字 → sha256`（格式：`<sha256>\t<bytes>\t<name>`）。
fn manifest_hashes(manifest: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(manifest);
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.contains('=') {
            continue;
        }
        let mut it = line.splitn(3, '\t');
        let (Some(sha), Some(_bytes), Some(name)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        out.insert(name.to_owned(), sha.to_owned());
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    // 测试自己重算一遍, 不调用被测方的内部函数 —— 否则是自证。
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[test]
fn bundle_contains_the_required_entries() {
    let scratch = Scratch::new("required");
    let logs = vec![("session.log".to_owned(), b"hello log".to_vec())];
    let report = export_diagnostics(
        &scratch.0,
        BundleInputs {
            state_json: Some(r#"{"state":"ok"}"#),
            config_json: Some(r#"{"cfg":1}"#),
            logs: &logs,
            crashes: &[],
            project: &[],
        },
    )
    .expect("导出必须成功");

    let entries = read_entries(&report.path);
    for required in [
        "MANIFEST.txt",
        "env.txt",
        "git.txt",
        "engine-state.json",
        "config.json",
        "logs/session.log",
    ] {
        assert!(entries.contains_key(required), "缺必需条目 {required}");
    }
    // 隐私默认: 没有 project/ 条目。
    assert!(
        !entries.keys().any(|k| k.starts_with("project/")),
        "默认不得包含工程文件"
    );
}

#[test]
fn manifest_sha256_matches_every_entry() {
    let scratch = Scratch::new("hashes");
    let crashes = vec![("crash.txt".to_owned(), b"boom".to_vec())];
    let report = export_diagnostics(
        &scratch.0,
        BundleInputs {
            state_json: Some("{}"),
            config_json: Some("{}"),
            logs: &[],
            crashes: &crashes,
            project: &[],
        },
    )
    .expect("导出必须成功");

    let entries = read_entries(&report.path);
    let manifest = entries.get("MANIFEST.txt").expect("MANIFEST").clone();
    let hashes = manifest_hashes(&manifest);
    assert!(!hashes.is_empty(), "MANIFEST 必须列出条目");

    // 判据 2: 逐项重算并与 MANIFEST 比对（MANIFEST 自己不列自己, 所以跳过它）。
    for (name, bytes) in &entries {
        if name == "MANIFEST.txt" {
            continue;
        }
        let want = hashes
            .get(name)
            .unwrap_or_else(|| panic!("MANIFEST 少了 {name} 的哈希"));
        assert_eq!(&sha256_hex(bytes), want, "{name} 的 sha256 不符");
    }
    // 反向: MANIFEST 里列的每一项都必须真的在包里（这条就是判据 5 的牙）。
    for name in hashes.keys() {
        assert!(entries.contains_key(name), "MANIFEST 列了不存在的 {name}");
    }
}

#[test]
fn bundle_is_redacted() {
    let scratch = Scratch::new("redact");
    let home = std::env::var("HOME").unwrap_or_default();
    assert!(!home.is_empty(), "本测试要求 HOME 存在");
    let config = format!(r#"{{"path":"{home}/Music/yeban","token":"SECRET"}}"#);
    let report = export_diagnostics(
        &scratch.0,
        BundleInputs {
            state_json: Some(&format!(r#"{{"dir":"{home}"}}"#)),
            config_json: Some(&config),
            logs: &[],
            crashes: &[],
            project: &[],
        },
    )
    .expect("导出必须成功");

    let entries = read_entries(&report.path);
    for (name, bytes) in &entries {
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.contains(&home),
            "{name} 里出现了真实 $HOME 路径（脱敏失败）"
        );
    }
    // 只有**确实带路径**的两个条目才应出现替换后的字面量;
    // `env.txt`/`git.txt`/`MANIFEST.txt` 本来就不含家目录 —— 对它们断言 "$HOME 必须在" 是错的判据。
    for name in ["engine-state.json", "config.json"] {
        let text = String::from_utf8_lossy(entries.get(name).expect("条目在包里"));
        assert!(
            text.contains("$HOME"),
            "{name} 应把真实家目录替换为字面量 $HOME"
        );
    }
}

#[test]
fn missing_manifest_entry_is_detectable() {
    // 判据 5（牙测）: 构造一个"包内条目"与"MANIFEST 记录"不一致的情形, 证明比对**能**失败。
    // 做法: 直接拿一个真实包的条目集合, 人为删掉一个, 再跑与判据 2 相同的比对 —— 必须红。
    let scratch = Scratch::new("tooth");
    let report = export_diagnostics(
        &scratch.0,
        BundleInputs {
            state_json: Some("{}"),
            config_json: Some("{}"),
            logs: &[],
            crashes: &[],
            project: &[],
        },
    )
    .expect("导出必须成功");
    let mut entries = read_entries(&report.path);
    let manifest = entries.get("MANIFEST.txt").expect("MANIFEST").clone();
    let hashes = manifest_hashes(&manifest);
    // 人为制造"应有却缺"：删掉 env.txt。
    entries.remove("env.txt");
    let missing: Vec<&String> = hashes
        .keys()
        .filter(|n| *n != "MANIFEST.txt" && !entries.contains_key(*n))
        .collect();
    assert!(
        !missing.is_empty(),
        "牙测失败: 删掉 env.txt 之后比对竟然没有发现 —— 判据没有牙"
    );
}
