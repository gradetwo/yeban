//! ⭐ **登记语料面的常驻判据**（本片：触发期随机族 `offset_random` / `amp_random`）。
//!
//! # 这条判据在防什么
//!
//! 新 opcode 的取值域如果比语料里**真实存在**的写法更窄，解析器会响亮报错 ——
//! 而后果不是"这一个 opcode 不生效"，是**整份乐器不可加载**（连它旁边正常的区域
//! 一起）。本片实测到一例：`freepats-button-accordion-hn` 的 `<group>` 段写着
//! `amp_random=-6`（规范表格是 `0 to 24`）⇒ 严格按表格拒收会让那份 34 个区域的
//! 乐器整份消失。
//!
//! 因此本判据钉两件事：
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | C1 | 语料里**没有任何**文件因为这两个 opcode 解析失败 | 1 397 份 `assets/samples/**/*.sfz` 的 `Err` 文本都不含这两个名字 |
//! | C2 | 语料里的负值**真的读进** `Region` | accordion 文件的末区域 `amp_random == -6.0` 且首个区域仍为缺省 0 |
//!
//! ⚠ **分母**：`assets/samples` 下 1 397 份 `.sfz`（`find` 读数）。仓库里另有 **1** 份
//! 测试夹具（`crates/yeban-app/tests/fixtures/sampler_c1.sfz`）在 `assets/` 之外，
//! ⛔ 不计入本判据的分母（语料普查的 1 398 是那两者之和）。

use std::path::{Path, PathBuf};

use yeban_sfz::{ParseLimits, parse_text};

/// 语料根（相对 crate 根：`crates/yeban-sfz/../../assets/samples`）。
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples")
}

/// 递归收集 `.sfz`，按路径排序（确定性）。
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|error| panic!("读目录 {dir:?}: {error}"));
    for entry in entries {
        let entry = entry.expect("目录项必须可读");
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "sfz") {
            out.push(path);
        }
    }
}

#[test]
fn c1_no_corpus_file_fails_because_of_the_random_families() {
    let mut files = Vec::new();
    collect(&corpus_root(), &mut files);
    files.sort();
    assert_eq!(
        files.len(),
        1_397,
        "分母变了 ⇒ 重新登记（1 397 是 assets/samples 下 .sfz 的 find 读数）"
    );
    let limits = ParseLimits::default();
    let mut failed = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("语料必须可读");
        if let Err(error) = parse_text(&text, &limits) {
            let message = error.to_string();
            // 这两族是本片新增的识别项 ⇒ 它们的取值域**必须**覆盖语料里的所有写法。
            assert!(
                !message.contains("offset_random") && !message.contains("amp_random"),
                "{path:?} 因为新增的随机族而不可加载：{message}"
            );
            failed.push((path.clone(), message));
        }
    }
    // 余下的失败是**既有**的（模式串/上限/其它 opcode），本判据不接管它们；
    // 打印出来是因为"这一行有没有变多"要靠下一次读数比。
    println!("语料解析失败（非本片原因）: {} 份", failed.len());
    for (path, message) in &failed {
        println!("  {:?}: {message}", path.file_name().unwrap_or_default());
    }
}

#[test]
fn c2_the_corpus_negative_amp_random_reaches_the_region() {
    let path =
        corpus_root().join("freepats-button-accordion-hn/PRESET Button Accordion HN tuned.sfz");
    let text = std::fs::read_to_string(&path).expect("登记语料必须可读");
    let instrument =
        parse_text(&text, &ParseLimits::default()).expect("负值 amp_random 不得让整份乐器不可加载");
    assert!(!instrument.regions().is_empty(), "该文件必须解析出区域");
    // 首区域在 `<group>` 之前 ⇒ 仍是缺省 0；末区域在组内 ⇒ 继承 −6。
    assert_eq!(instrument.regions()[0].amp_random, 0.0);
    let last = instrument.regions().last().expect("非空");
    assert_eq!(last.amp_random, -6.0, "组级 amp_random=-6 必须继承到区域");
    // 同组还写着 `offset_random=10` ⇒ 也要真的读进。
    assert_eq!(last.offset_random, 10);
}
