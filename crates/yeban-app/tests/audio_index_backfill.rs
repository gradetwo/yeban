//! 判据：**装载时按池字节补登 `project.assets`**（T-FIX-329 的兼容面）。
//!
//! ## 要证的句子
//!
//! 今天 main 产出的带音频 `.yeban` 只写 CAS 池、不写 `project.assets`（`--import-audio`
//! 与录音落地都如此）。T-FIX-329 的写前核对要求两者都在 ⇒ 这类文件"能打开、存不回去"。
//! 本判据证：**打开一次就补登索引，保存随之成功，写出的文件自己就是索引齐全的**
//! （再打开不需要再补，字节也不漂移）。
//!
//! 同时证两条反向腿，确保 329 的硬化没被顺手削弱：
//!
//! - 池里**没有**字节的真缺口 ⇒ 补登不制造条目，保存仍然 `MissingAsset`；
//! - 池里**没有被引用**的多余资产 ⇒ 不补登，继续出现在 `AssetAudit::unindexed_pool`。
//!
//! ## 去掉本片改动 ⇒ 哪条红
//!
//! - 删掉 `open_project_archive` 里的 `backfill_audio_index_from_pool` 调用 ⇒
//!   第 1 条在 `save_archive_file` 上拿到 `MissingAsset`（正是本单要修的故障）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use yeban_app::open::{audit_project_assets, open_project_archive};
use yeban_app::save::{SaveError, save_archive_file};
use yeban_model::AssetHash;
use yeban_model::container::{ContainerLimits, write_project_container};
use yeban_model::project::MediaKind;
use yeban_model::samples::{demo_project, demo_project_assets};

/// 每个用例独立的临时目录（并发跑不会互相踩）。
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-audio-index-backfill-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 造一份**今天 main 会产出**的容器：音频片段引用 kick，池里有字节，`project.assets` 空。
///
/// 夹具不手写假字节：kick 的字节取自 [`demo_project_assets`]（与工程片段同源）。
fn legacy_container_with_pool_bytes() -> (Vec<u8>, AssetHash, u64) {
    let mut project = demo_project();
    let assets = demo_project_assets();
    let (hash, bytes) = assets[0].clone();
    let byte_len = u64::try_from(bytes.len()).expect("字节长度");
    project.assets.clear();
    assert!(
        project.assets.is_empty(),
        "夹具前提：索引必须是空的（模拟只写池的产出）"
    );

    let pool: BTreeMap<AssetHash, Vec<u8>> = assets.into_iter().collect();
    let container =
        write_project_container(&project, b"legacy-history", &pool).expect("写出只带池的容器");
    (container, hash, byte_len)
}

/// 判据 1：池有字节、索引缺项 ⇒ 打开即补登，随后保存成功，且写出的文件自洽。
#[test]
fn legacy_archive_backfills_the_index_on_open_and_then_saves() {
    let (container, kick_hash, kick_len) = legacy_container_with_pool_bytes();
    let dir = scratch_dir("heal");

    // —— 打开：只缺索引不得阻断打开；随后**先打保存这一枪**（本单的故障点：
    // 没有补登时这里拿到的就是 `MissingAsset`，红对照因此咬在"存不回去"上）。
    let archive = open_project_archive(&container, &ContainerLimits::default())
        .expect("只缺索引不得阻断打开");
    let first = dir.join("healed.yeban");
    let report = save_archive_file(&archive, &first).expect("补登后必须能保存");
    assert!(report.bytes > 0);

    // —— 补登恰好 1 条，且能看见它改了什么（load_notes 不是可选项）。
    assert_eq!(
        archive.load_notes.len(),
        1,
        "补登必须留下一条装载留痕: {:?}",
        archive.load_notes
    );
    assert!(
        archive.load_notes[0].contains("资产索引补登：1"),
        "留痕要点名补了几条: {:?}",
        archive.load_notes
    );

    let entry = archive
        .project
        .assets
        .get(&kick_hash)
        .expect("被引用的 kick 必须补登出索引项");
    assert_eq!(entry.hash, kick_hash);
    assert_eq!(entry.byte_len, kick_len, "byte_len 必须取自池");
    assert_eq!(entry.media_kind, MediaKind::Audio);
    assert!(entry.original_path.is_empty(), "不写机器相关路径");
    assert_eq!(entry.license, "unknown", "许可未知就写 unknown，不编");

    let audit = audit_project_assets(&archive);
    assert!(audit.missing.is_empty(), "补登不得掩盖真缺口");
    assert!(
        audit.unindexed_pool.is_empty(),
        "被引用资产的缺索引已补上: {:?}",
        audit.unindexed_pool
    );

    // —— 写出的文件自己就该是索引齐全的（重开不需要再补，字节也不漂移）。
    let written = std::fs::read(&first).expect("读回写出的文件");
    let reopened =
        open_project_archive(&written, &ContainerLimits::default()).expect("写出的文件必须能开");
    assert!(
        reopened.load_notes.is_empty(),
        "写出的文件自己索引齐全，重开不该再补: {:?}",
        reopened.load_notes
    );
    assert!(
        reopened.project.assets.contains_key(&kick_hash),
        "索引必须真的写进容器"
    );

    // —— 再存一次：字节不漂移（补登是幂等的，不引入随机/机器信息）。
    let second = dir.join("healed-again.yeban");
    save_archive_file(&reopened, &second).expect("第二次保存");
    assert_eq!(
        std::fs::read(&second).expect("读回第二次"),
        written,
        "同一归档两次写出必须逐字节相同"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 2（反向腿）：池里也没有字节 ⇒ 补登不制造条目，329 的拒绝照旧。
#[test]
fn pool_without_bytes_is_still_refused_after_open() {
    let mut project = demo_project();
    project.assets.clear();
    let container = write_project_container(&project, b"legacy-history", &BTreeMap::new())
        .expect("写出没有池的容器");
    let dir = scratch_dir("dangling");
    let path = dir.join("dangling.yeban");

    let archive = open_project_archive(&container, &ContainerLimits::default())
        .expect("缺字节不阻断打开（330 的读数语义）");
    assert!(
        archive.project.assets.is_empty(),
        "没有字节就不许凭空造索引项"
    );
    assert!(
        archive
            .load_notes
            .iter()
            .any(|note| note.contains("资产缺口")),
        "真缺口要如实留痕: {:?}",
        archive.load_notes
    );

    let error = save_archive_file(&archive, &path).expect_err("缺字节的音频引用必须拒绝");
    assert!(
        matches!(error, SaveError::MissingAsset { .. }),
        "实际: {error:?}"
    );
    assert!(!path.exists(), "⛔ 一个字节都不写");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 3（边界）：池里没被任何片段引用的多余资产**不**补登，继续进审计读数。
#[test]
fn unreferenced_pool_assets_are_not_backfilled() {
    let project = demo_project();
    let mut pool: BTreeMap<AssetHash, Vec<u8>> = demo_project_assets().into_iter().collect();
    let extra_bytes = b"never-referenced-by-any-clip".to_vec();
    let extra_hash = AssetHash::of_bytes(&extra_bytes);
    pool.insert(extra_hash.clone(), extra_bytes);
    let container = write_project_container(&project, b"", &pool).expect("写出带多余资产的容器");

    let archive =
        open_project_archive(&container, &ContainerLimits::default()).expect("多余资产不阻断打开");
    assert!(
        archive.load_notes.is_empty(),
        "没有音频片段缺索引 ⇒ 不该有补登留痕: {:?}",
        archive.load_notes
    );
    assert!(
        !archive.project.assets.contains_key(&extra_hash),
        "没被引用的资产不该被补登"
    );
    let audit = audit_project_assets(&archive);
    assert_eq!(
        audit.unindexed_pool,
        vec![extra_hash],
        "多余资产必须继续出现在审计里"
    );
}
