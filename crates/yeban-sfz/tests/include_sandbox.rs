//! `#include` 沙箱 / 展开顺序 / 宏跨文件生效的集成测试。
//!
//! 这些是**安全红线**判据：绝对路径、`..` 逃逸、符号链接逃逸、递归成环、深度炸弹
//! 都必须变成可读的 [`SfzError`]，而不是读到基准目录之外的文件。
//!
//! 规范来源: ROAD-M2-005（零拷贝 SFZ 解析器）、`#include` 语义见
//! <https://sfzformat.com/opcodes/include/>。

use std::fs;
use std::path::Path;

use yeban_sfz::{IncludeResolver, ParseLimits, SfzError, parse_sources};

mod support;

use support::TempDir;

fn write_file(root: &Path, name: &str, contents: &str) {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(&path, contents).expect("write fixture");
}

fn resolve_and_parse(root: &Path, entry: &str) -> Result<yeban_sfz::Instrument<'static>, SfzError> {
    // 用 Box::leak 让源文本活到 'static，便于在测试里返回 Instrument。
    // 仅在测试里这么做，生产代码由调用方持有 Vec<SfzSource>。
    let limits = ParseLimits::default();
    let resolver = IncludeResolver::new(root, limits).expect("base dir");
    let sources = resolver.resolve(entry)?;
    let leaked: &'static [yeban_sfz::SfzSource] = Box::leak(sources.into_boxed_slice());
    parse_sources(leaked, &limits)
}

#[test]
fn include_is_expanded_in_place_in_paste_order() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "inc.sfz", "<region>sample=inc.wav\n");
    write_file(
        root,
        "main.sfz",
        "<group>key=36\n#include \"inc.sfz\"\n<region>sample=main.wav\n",
    );

    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    let samples: Vec<&str> = instrument
        .regions()
        .iter()
        .map(|region| region.sample.as_ref())
        .collect();
    // 被包含文件在原位展开：inc.wav 在 main.wav 之前。
    assert_eq!(samples, vec!["inc.wav", "main.wav"]);
    // `<group>key=36` 对被包含片段同样生效（继承跨片段延续）。
    assert_eq!(instrument.regions()[0].lokey, 36);
    assert_eq!(instrument.regions()[0].hikey, 36);
}

#[test]
fn define_in_main_file_applies_to_included_file() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "inc.sfz", "<region>key=$KICK sample=inc.wav\n");
    write_file(root, "main.sfz", "#define $KICK 36\n#include \"inc.sfz\"\n");

    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    assert_eq!(instrument.regions()[0].lokey, 36);
    assert_eq!(instrument.regions()[0].hikey, 36);
}

#[test]
fn relative_include_inside_a_subdirectory_resolves_from_the_base_dir() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "main.sfz", "#include \"parts/a.sfz\"\n");

    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    assert_eq!(instrument.regions()[0].sample, "a.wav");
}

#[test]
fn absolute_include_is_rejected() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "main.sfz", "#include \"/etc/hosts.sfz\"\n");

    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeAbsolutePath { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn parent_directory_escape_is_rejected() {
    let dir = TempDir::new("sfz");
    let root = dir.path().join("base");
    fs::create_dir_all(&root).expect("base dir");
    write_file(dir.path(), "outside.sfz", "<region>sample=x.wav\n");
    write_file(&root, "main.sfz", "#include \"../outside.sfz\"\n");

    let error = resolve_and_parse(&root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeEscape { .. }),
        "unexpected: {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected_even_though_the_path_looks_relative() {
    let dir = TempDir::new("sfz");
    let base = dir.path().join("base");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&base).expect("base dir");
    fs::create_dir_all(&outside).expect("outside dir");
    write_file(&outside, "secret.sfz", "<region>sample=secret.wav\n");
    std::os::unix::fs::symlink(outside.join("secret.sfz"), base.join("link.sfz"))
        .expect("create symlink");
    write_file(&base, "main.sfz", "#include \"link.sfz\"\n");

    let error = resolve_and_parse(&base, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeEscape { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn include_cycle_is_detected() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "a.sfz", "#include \"b.sfz\"\n");
    write_file(root, "b.sfz", "#include \"a.sfz\"\n");

    let error = resolve_and_parse(root, "a.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeCycle { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn include_depth_limit_is_enforced() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    for index in 0..6 {
        write_file(
            root,
            &format!("f{index}.sfz"),
            &format!("#include \"f{}.sfz\"\n", index + 1),
        );
    }
    write_file(root, "f6.sfz", "<region>sample=deep.wav\n");

    let limits = ParseLimits {
        max_include_depth: 3,
        ..ParseLimits::default()
    };
    let resolver = IncludeResolver::new(root, limits).expect("base dir");
    let error = resolver.resolve("f0.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeDepthExceeded { limit: 3 }),
        "unexpected: {error:?}"
    );
}

#[test]
fn include_file_count_limit_is_enforced() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    for index in 0..4 {
        write_file(root, &format!("p{index}.sfz"), "<region>sample=x.wav\n");
    }
    write_file(
        root,
        "main.sfz",
        "#include \"p0.sfz\"\n#include \"p1.sfz\"\n#include \"p2.sfz\"\n#include \"p3.sfz\"\n",
    );

    let limits = ParseLimits {
        max_include_files: 2,
        ..ParseLimits::default()
    };
    let resolver = IncludeResolver::new(root, limits).expect("base dir");
    let error = resolver.resolve("main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeCountExceeded { limit: 2 }),
        "unexpected: {error:?}"
    );
}

#[test]
fn include_must_be_double_quoted() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "inc.sfz", "<region>sample=inc.wav\n");
    write_file(root, "main.sfz", "#include inc.sfz\n");

    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeNotQuoted { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn include_extension_must_be_sfz_or_sfzh() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "sample.wav", "not an sfz");
    write_file(root, "main.sfz", "#include \"sample.wav\"\n");

    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeUnsupportedExtension { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn missing_include_is_reported() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "main.sfz", "#include \"nope.sfz\"\n");

    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeNotFound { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn non_utf8_include_is_reported_not_guessed() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    fs::write(root.join("bad.sfz"), [0xff, 0xfe, 0x00, 0x80]).expect("write bytes");
    write_file(root, "main.sfz", "#include \"bad.sfz\"\n");

    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::NotUtf8 { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn glob_include_matches_sorted_and_ignores_other_extensions() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "parts/z.sfz", "<region>sample=z.wav\n");
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "parts/m.sfzh", "<region>sample=m.wav\n");
    write_file(root, "parts/skip.wav", "not sfz");
    write_file(
        root,
        "main.sfz",
        "#include \"parts/*.sfz\"\n#include \"parts/*.sfzh\"\n",
    );

    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    let samples: Vec<&str> = instrument
        .regions()
        .iter()
        .map(|region| region.sample.as_ref())
        .collect();
    // 目录枚举顺序不参与：按路径字典序确定性排序。
    assert_eq!(samples, vec!["a.wav", "z.wav", "m.wav"]);
}

#[test]
fn glob_include_without_matches_is_an_error() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "main.sfz", "#include \"parts/*.sfz\"\n");

    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeNoMatch { .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn recursive_glob_is_bounded_and_ordered() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "kit/a.sfz", "<region>sample=deep_a.wav\n");
    write_file(root, "kit/sub/b.sfz", "<region>sample=deep_b.wav\n");
    write_file(root, "main.sfz", "#include \"kit/**/*.sfz\"\n");

    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    let samples: Vec<&str> = instrument
        .regions()
        .iter()
        .map(|region| region.sample.as_ref())
        .collect();
    assert_eq!(samples, vec!["deep_a.wav", "deep_b.wav"]);
}
