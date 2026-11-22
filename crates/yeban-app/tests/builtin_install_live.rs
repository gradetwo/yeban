//! ⭐ 真包判据（`#[ignore]`）：用**真实的上游发布包**验证三段逻辑——
//! 「下载大小口径」「发布包不抹外层目录」「主 SFZ 就位」。
//!
//! 手工跑：
//! ```text
//! # ① 走真实下载器（需能连 github.com / release-assets）
//! cargo test --release -p yeban-app --test builtin_install_live -- --ignored --nocapture
//!
//! # ② 只验装包与落位（先用 curl 把包下到本地）
//! YEBAN_TEST_RELEASE_ZIP=/tmp/emily.zip \
//!   cargo test --release -p yeban-app --test builtin_install_live local_release -- --ignored --nocapture
//! ```
//!
//! ⚠ 包约 100 MB，因此两条都不进常驻 CI。

use std::path::Path;

use yeban_app::builtin_sounds::{MANIFEST_RELATIVE, download_builtin, parse_manifest};
use yeban_app::sound_library::{CatalogPack, SoundRoots, install_pack_archive};

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("仓库根")
        .to_path_buf()
}

fn emily() -> yeban_app::builtin_sounds::BuiltinSound {
    let text = std::fs::read_to_string(repo_root().join(MANIFEST_RELATIVE)).expect("读登记表");
    parse_manifest(&text)
        .expect("解析登记表")
        .into_iter()
        .find(|s| s.id == "karoryfer-emilyguitar")
        .expect("登记表里应有 emily")
}

fn roots() -> SoundRoots {
    let library_root = std::env::temp_dir().join("yeban-live-sounds");
    let _ = std::fs::remove_dir_all(&library_root);
    SoundRoots {
        library_root,
        builtin_root: repo_root().join("assets/samples"),
    }
}

/// ① 走真实下载器（网络条件好的机器上跑）。
#[test]
#[ignore = "真下载约 100 MB；手工跑"]
fn installs_emily_from_its_release_zip() {
    let sound = emily();
    let roots = roots();
    let mut seen_mb = 0u64;
    let installed = download_builtin(&sound, &roots, &mut |done, _total| {
        let mb = done / 20_000_000;
        if mb != seen_mb {
            seen_mb = mb;
            eprintln!("下载 {} MB", mb * 20);
        }
    })
    .expect("发布包应能装成功");

    let sfz = installed.location.join(&sound.sfz);
    assert!(
        sfz.is_file(),
        "主 SFZ 必须就位：{}（实际落位 {}）",
        sfz.display(),
        installed.location.display()
    );
    assert!(
        installed.location.join("Emilyguitar").is_dir(),
        "发布包不应被抹平外层目录"
    );
    eprintln!(
        "installed at {} · sfz={} · size={} B",
        installed.location.display(),
        sound.sfz,
        installed.size_bytes
    );
}

/// ② 只验装包与落位：`YEBAN_TEST_RELEASE_ZIP` 指向已下好的发布包。
///
/// 这一条**同时**证明登记表里的 `archive.sha256` 与真实发布包一致
/// （`install_pack_archive` 对非空 sha 是硬校验），因此它不只是"布局对了"，
/// 而是"这一份字节就是登记的那一份"。
#[test]
#[ignore = "需要先手工下载发布包并给出 YEBAN_TEST_RELEASE_ZIP"]
fn installs_a_local_release_zip_without_flattening() {
    let Ok(zip) = std::env::var("YEBAN_TEST_RELEASE_ZIP") else {
        eprintln!("跳过：未给 YEBAN_TEST_RELEASE_ZIP");
        return;
    };
    let sound = emily();
    assert!(
        !sound.release_zip.is_empty(),
        "发布包条目必须带 release_zip（否则会退回 commit 归档，布局就错了）"
    );
    let pack = CatalogPack {
        id: sound.id.clone(),
        name: sound.name.clone(),
        description: sound.download_note(),
        version: "发布包".to_owned(),
        // ⛔ 不用 `total_bytes`（解压后口径）；这一条不校验大小，只校验 sha。
        size_bytes: 0,
        sha256: sound.release_sha256.clone(),
        url: sound.release_zip.clone(),
    };
    let roots = roots();
    let installed = install_pack_archive(&pack, Path::new(&zip), &roots).expect("装包应成功");

    // 发布包顶层就是 `Emilyguitar/` ⇒ 不抹；主 SFZ 必须在。
    assert!(
        installed.location.join("Emilyguitar").is_dir(),
        "落位后应保留 Emilyguitar/ 这一层"
    );
    assert!(
        installed.location.join(&sound.sfz).is_file(),
        "主 SFZ 必须就位：{}",
        sound.sfz
    );
    eprintln!(
        "installed at {} · sfz={} · size={} B",
        installed.location.display(),
        sound.sfz,
        installed.size_bytes
    );
}
