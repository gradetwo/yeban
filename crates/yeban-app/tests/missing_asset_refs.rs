//! T-FIX-330：Audio 片段引用的资产不在资产池 ⇒ 生产打开（严格模式）响亮失败，
//! 默认读法给出可显示 warning；池里多出的资产只作读数，不阻断打开。
//!
//! ⚠ 只测打开这一层，不建窗口、不跑 Slint 平台 ⇒ 没有 `LivePort` 的单测约束。

use std::collections::BTreeMap;

use yeban_app::open::{
    OpenError, ProjectOpenOptions, audit_project_assets, open_project_archive_file,
    open_project_document_file,
};
use yeban_model::container::write_project_container;
use yeban_model::samples::demo_project;

fn write_fixture(
    name: &str,
    assets: &BTreeMap<yeban_model::AssetHash, Vec<u8>>,
) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-missing-asset-refs-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let bytes = write_project_container(&demo_project(), b"", assets).expect("写出真容器");
    let path = dir.join(format!("{name}.yeban"));
    std::fs::write(&path, bytes).expect("写临时容器");
    path
}

#[test]
fn missing_audio_asset_is_a_strict_open_error_and_a_default_load_note() {
    let path = write_fixture("missing", &BTreeMap::new());

    let strict = ProjectOpenOptions {
        strict_missing_assets: true,
        ..ProjectOpenOptions::default()
    };
    match open_project_document_file(&path, &strict) {
        Err(OpenError::MissingAsset {
            hash,
            referenced_by,
            missing_count,
        }) => {
            assert_eq!(missing_count, 1, "demo 只有一个被引用的 kick");
            assert!(
                referenced_by
                    .iter()
                    .any(|label| label.starts_with("track:")),
                "回执必须能指到轨/摆放: {referenced_by:?}"
            );
            assert!(
                referenced_by.iter().any(|label| label.starts_with("clip:")),
                "回执必须能指到片段: {referenced_by:?}"
            );
            assert_eq!(hash.to_string().len(), 64, "必须是规范 CAS 哈希");
        }
        other => panic!("严格模式期望 MissingAsset，实测 {other:?}"),
    }

    // 默认读法不阻断（兼容旧工程），但缺口必须进 `load_notes` 与 `asset_audit`。
    let opened = open_project_document_file(&path, &ProjectOpenOptions::default())
        .expect("默认读法必须能打开");
    assert_eq!(opened.asset_audit.missing.len(), 1);
    assert!(
        opened
            .asset_audit
            .missing
            .iter()
            .any(|gap| gap.hash.to_string().len() == 64),
        "审计必须给出缺失哈希"
    );
    assert!(
        opened
            .archive
            .load_notes
            .iter()
            .any(|note| note.contains("资产缺口") && note.contains("缺失资产")),
        "缺口必须写进 load_notes: {:?}",
        opened.archive.load_notes
    );
}

#[test]
fn extra_pool_asset_is_a_reading_not_a_rejection() {
    let mut assets: BTreeMap<yeban_model::AssetHash, Vec<u8>> =
        yeban_model::samples::demo_project_assets()
            .into_iter()
            .collect();
    let extra = b"pool-extra-never-referenced".to_vec();
    let extra_hash = yeban_model::AssetHash::of_bytes(&extra);
    assets.insert(extra_hash.clone(), extra);
    let path = write_fixture("extra", &assets);

    let opened = open_project_document_file(
        &path,
        &ProjectOpenOptions {
            strict_missing_assets: true,
            ..ProjectOpenOptions::default()
        },
    )
    .expect("池里多出资产不得阻断打开");
    let audit = &opened.asset_audit;
    assert_eq!(audit.pool, 2);
    assert_eq!(audit.project_index, 1);
    assert_eq!(audit.unindexed_pool, vec![extra_hash.clone()]);
    assert_eq!(audit.unreferenced_pool, vec![extra_hash]);

    // 纯审计函数对同一个归档给同一读数（UI 与 CLI 共用一份口径）。
    let direct = audit_project_assets(&opened.archive);
    assert_eq!(direct, *audit);
}

#[test]
fn missing_assets_are_detected_from_embedded_samples_too() {
    let mut project = demo_project();
    let fake = yeban_model::AssetHash::of_bytes(b"embedded-sample-without-bytes");
    project
        .embedded_samples
        .insert("samples/fake.sfz".to_owned(), fake.to_string());
    let dir = std::env::temp_dir().join(format!("yeban-embedded-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let bytes = write_project_container(&project, b"", &BTreeMap::new()).expect("写容器");
    let path = dir.join("embedded.yeban");
    std::fs::write(&path, bytes).expect("写临时容器");

    let archive =
        open_project_archive_file(&path, &ProjectOpenOptions::default()).expect("默认读法能打开");
    let audit = audit_project_assets(&archive);
    assert_eq!(audit.missing.len(), 2, "kick + 内嵌样本 各缺一个");
    assert!(
        audit.missing.iter().any(|gap| gap
            .referenced_by
            .iter()
            .any(|label| label.starts_with("embedded_samples:"))),
        "内嵌样本引用必须被枚举: {:?}",
        audit.missing
    );

    assert_eq!(archive.assets.len(), 0);
}
