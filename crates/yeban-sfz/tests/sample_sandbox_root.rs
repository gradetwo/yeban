//! ⭐ T-FIX-126：`sample=` 的**根内**解析口径（词法判 != 文件系统判）。
//!
//! 本判据钉两件事：
//! ① `validate_sample_path` 拒绝 Windows **驱动器相对**路径（`C:foo.wav`）——旧判据只拦
//!    `C:/x`（第 3 字节是 `/`），于是 `C:foo.wav` 从"绝对"缝里漏过去；
//! ② `resolve_sample_under_root` 是唯一能拦住**符号链接**与**根外绝对路径**的口径：
//!    规范化后必须以规范化 root 为前缀，⛔ 不靠"看见 `..` 才算逃逸"。

use std::path::PathBuf;

use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::{resolve_sample_under_root, validate_sample_path};

/// 一次性临时根：进程 id ＋ 纳秒 ＋ tag，⛔ 不引第三方 crate。
fn temp_root(tag: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "yeban-sfz-root-{tag}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    root
}

#[test]
fn drive_relative_sample_paths_are_rejected_lexically() {
    for sample in ["C:foo.wav", "c:foo.wav", "Z:..\\x.wav", "C:"] {
        let error = validate_sample_path(sample)
            .expect_err(&format!("`{sample}` 必须被拒绝（驱动器相对路径）"));
        assert!(
            matches!(error, SfzError::SampleEscape { .. }),
            "`{sample}` 应落 SampleEscape，实际 {error:?}"
        );
    }
    assert!(validate_sample_path("samples/foo.wav").is_ok());
    assert!(matches!(
        validate_sample_path("C:/tmp/foo.wav").expect_err("drive-absolute 必须被拒绝"),
        SfzError::SampleAbsolutePath { .. }
    ));
}

#[test]
fn relative_sample_resolves_inside_root() {
    let root = temp_root("rel");
    std::fs::create_dir_all(root.join("samples")).expect("mkdir samples");
    let file = root.join("samples/foo.wav");
    std::fs::write(&file, b"RIFF").expect("write sample");

    let resolved =
        resolve_sample_under_root(&root, "samples/foo.wav").expect("根内相对路径必须通过");
    assert_eq!(resolved, file.canonicalize().expect("canonicalize sample"));
    assert!(resolved.starts_with(root.canonicalize().expect("canonicalize root")));
}

#[test]
fn absolute_sample_inside_root_still_loads() {
    let root = temp_root("abs");
    let file = root.join("a.wav");
    std::fs::write(&file, b"RIFF").expect("write sample");
    let absolute = file.canonicalize().expect("canonicalize");
    let resolved = resolve_sample_under_root(&root, &absolute.display().to_string())
        .expect("根内绝对路径必须通过");
    assert_eq!(resolved, absolute);
}

#[test]
fn absolute_sample_outside_root_is_escape() {
    let root = temp_root("absout");
    let outside = temp_root("outside").join("evil.wav");
    std::fs::write(&outside, b"RIFF").expect("write outside sample");
    let error = resolve_sample_under_root(&root, &outside.display().to_string())
        .expect_err("根外绝对路径必须被拒绝");
    assert!(
        matches!(error, SfzError::SampleEscape { .. }),
        "实际 {error:?}"
    );
}

#[test]
fn symlink_escape_is_rejected() {
    #[cfg(unix)]
    {
        let root = temp_root("link");
        let outside = temp_root("linked-out").join("evil.wav");
        std::fs::write(&outside, b"RIFF").expect("write outside sample");
        std::os::unix::fs::symlink(&outside, root.join("link.wav")).expect("symlink");
        let error =
            resolve_sample_under_root(&root, "link.wav").expect_err("符号链接指向根外必须被拒绝");
        assert!(
            matches!(error, SfzError::SampleEscape { .. }),
            "实际 {error:?}"
        );
    }
}

#[test]
fn dotdot_and_missing_files_are_loud() {
    let root = temp_root("dotdot");
    assert!(matches!(
        resolve_sample_under_root(&root, "../etc/passwd").expect_err("`..` 必须被拒绝"),
        SfzError::SampleEscape { .. }
    ));
    assert!(matches!(
        resolve_sample_under_root(&root, "samples/missing.wav").expect_err("缺文件必须报错"),
        SfzError::SampleInvalidPath { .. }
    ));
}
