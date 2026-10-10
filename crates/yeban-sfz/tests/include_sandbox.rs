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

// ---------------------------------------------------------------------------
// 配额边界、目录 / 隐藏项 / 符号链接过滤与行号（沙箱的另一半契约）
// ---------------------------------------------------------------------------

/// 在 `root` 下建一条 `f0.sfz -> f1.sfz -> ...` 的 include 链，末个文件里放一个 region。
fn build_chain(root: &Path, count: usize) {
    for index in 0..count {
        let body = if index + 1 == count {
            "<region>sample=deep.wav\n".to_string()
        } else {
            format!("#include \"f{}.sfz\"\n", index + 1)
        };
        write_file(root, &format!("f{index}.sfz"), &body);
    }
}

#[test]
fn include_depth_limit_is_exact() {
    // 深度是闭上界：`max_include_depth = 3` 恰好放行 3 个文件（深度 0..=2），
    // 第 4 个文件（深度 3）才 `Err`。
    let limits = ParseLimits {
        max_include_depth: 3,
        ..ParseLimits::default()
    };
    let dir = TempDir::new("sfz");
    let root = dir.path();
    build_chain(root, 3);
    IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("f0.sfz")
        .expect("exactly the depth cap is accepted");

    let deep = TempDir::new("sfz");
    let deep_root = deep.path();
    build_chain(deep_root, 4);
    let error = IncludeResolver::new(deep_root, limits)
        .expect("base dir")
        .resolve("f0.sfz")
        .expect_err("one level past the cap");
    assert!(
        matches!(error, SfzError::IncludeDepthExceeded { limit: 3 }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn include_file_count_limit_is_exact() {
    // 文件计数**含入口文件本身**：`max_include_files = 2` 恰好放行「入口 + 1 个被包含文件」。
    let limits = ParseLimits {
        max_include_files: 2,
        ..ParseLimits::default()
    };
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "p0.sfz", "<region>sample=p0.wav\n");
    write_file(root, "p1.sfz", "<region>sample=p1.wav\n");
    write_file(root, "one.sfz", "#include \"p0.sfz\"\n");
    IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("one.sfz")
        .expect("entry plus one include is exactly the cap");

    write_file(
        root,
        "two.sfz",
        "#include \"p0.sfz\"\n#include \"p1.sfz\"\n",
    );
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("two.sfz")
        .expect_err("one file past the cap");
    assert!(
        matches!(error, SfzError::IncludeCountExceeded { limit: 2 }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn an_include_of_a_directory_is_not_a_file() {
    // `canonical_include` 的 `is_file` 断言：目录即使以 `.sfz` 结尾也必须变成明确的
    // `IncludeNotAFile`，不能落进 `fs::read` 的 I/O 错误（那是不可区分的诊断）。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    fs::create_dir_all(root.join("d.sfz")).expect("create dir");
    write_file(root, "main.sfz", "#include \"d.sfz\"\n");
    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeNotAFile { .. }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn globs_skip_dot_entries() {
    // `entries` 的口径是「可见项」：点开头的条目既不参与匹配也不参与排序。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "parts/.hidden.sfz", "<region>sample=hidden.wav\n");
    write_file(root, "main.sfz", "#include \"parts/*.sfz\"\n");
    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    let samples: Vec<&str> = instrument
        .regions()
        .iter()
        .map(|region| region.sample.as_ref())
        .collect();
    assert_eq!(samples, vec!["a.wav"]);
}

#[test]
fn a_recursive_glob_does_not_descend_into_a_symlinked_directory() {
    // 递归 glob 与单文件 include 同一条「不跟随符号链接」口径：链接目录里的匹配项
    // 不得进结果集（即使链接目标仍在沙箱内），否则 `**` 会绕开 `is_symlink` 过滤。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "kit/a.sfz", "<region>sample=direct.wav\n");
    write_file(root, "elsewhere/b.sfz", "<region>sample=through_link.wav\n");
    std::os::unix::fs::symlink(root.join("elsewhere"), root.join("kit/linked"))
        .expect("create dir symlink");
    write_file(root, "main.sfz", "#include \"kit/**/*.sfz\"\n");
    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    let samples: Vec<&str> = instrument
        .regions()
        .iter()
        .map(|region| region.sample.as_ref())
        .collect();
    assert_eq!(samples, vec!["direct.wav"]);
}

#[test]
fn glob_match_quota_is_exact() {
    // 匹配数配额是闭上界：恰好 `n` 个匹配放行，第 `n + 1` 个才 `Err`。
    let limits = ParseLimits {
        max_glob_matches: 2,
        ..ParseLimits::default()
    };
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "parts/b.sfz", "<region>sample=b.wav\n");
    write_file(root, "main.sfz", "#include \"parts/*.sfz\"\n");
    IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("main.sfz")
        .expect("exactly the match cap is accepted");

    write_file(root, "parts/c.sfz", "<region>sample=c.wav\n");
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("main.sfz")
        .expect_err("one match past the cap");
    assert!(
        matches!(error, SfzError::GlobMatchesExceeded { limit: 2, .. }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn glob_scanned_quota_is_exact() {
    // 扫描配额（`read_dir` 迭代次数）同样是闭上界：扫到第 `n + 1` 个目录项才 `Err`。
    let limits = ParseLimits {
        max_glob_scanned: 2,
        ..ParseLimits::default()
    };
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "parts/b.sfz", "<region>sample=b.wav\n");
    write_file(root, "main.sfz", "#include \"parts/*.sfz\"\n");
    IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("main.sfz")
        .expect("two directory entries are exactly the cap");

    write_file(root, "parts/c.sfz", "<region>sample=c.wav\n");
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("main.sfz")
        .expect_err("one entry past the cap");
    assert!(
        matches!(error, SfzError::GlobScanExceeded { limit: 2, .. }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn a_backslash_separator_is_normalized() {
    // 词法检查把 `\` 归一成 `/`（Windows 写法的 include 在登记语料里存在）。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "sub/a.sfz", "<region>sample=sub.wav\n");
    write_file(root, "main.sfz", "#include \"sub\\a.sfz\"\n");
    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    assert_eq!(instrument.regions()[0].sample, "sub.wav");
}

#[test]
fn the_resolver_define_cap_boundary_is_exact() {
    // resolver 侧的 `#define` 表与解析器侧是**两条**独立上限（各自检查），
    // 因此各自的闭上界都要钉住。
    let limits = ParseLimits {
        max_defines: 2,
        ..ParseLimits::default()
    };
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(
        root,
        "two.sfz",
        "#define $A 1\n#define $B 2\n<region>sample=a.wav\n",
    );
    IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("two.sfz")
        .expect("exactly two defines fit");

    write_file(
        root,
        "three.sfz",
        "#define $A 1\n#define $B 2\n#define $C 3\n<region>sample=a.wav\n",
    );
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("three.sfz")
        .expect_err("one define past the cap");
    assert!(
        matches!(error, SfzError::TooManyDefines { limit: 2 }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn the_source_byte_cap_boundary_is_exact() {
    // 源文件字节上限是闭上界：恰好等长放行，多 1 字节才 `Err`。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    let body = "<region>sample=a.wav\n";
    write_file(root, "exact.sfz", body);
    let exact = ParseLimits {
        max_source_bytes: body.len(),
        ..ParseLimits::default()
    };
    IncludeResolver::new(root, exact)
        .expect("base dir")
        .resolve("exact.sfz")
        .expect("a file of exactly the cap is accepted");

    let one_less = ParseLimits {
        max_source_bytes: body.len() - 1,
        ..ParseLimits::default()
    };
    let error = IncludeResolver::new(root, one_less)
        .expect("base dir")
        .resolve("exact.sfz")
        .expect_err("one byte past the cap");
    assert!(
        matches!(
            error,
            SfzError::SourceTooLarge { len, limit, .. }
                if len == body.len() && limit == body.len() - 1
        ),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn the_resolver_line_byte_cap_boundary_is_exact() {
    // 行上限在 resolver 的文件扫描循环里是**第二处**独立检查（与 `Parser::run` 同口径），
    // 边界同样含行尾换行。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    let body = "<region>sample=a.wav\n";
    write_file(root, "exact.sfz", body);
    let exact = ParseLimits {
        max_line_bytes: body.len(),
        ..ParseLimits::default()
    };
    IncludeResolver::new(root, exact)
        .expect("base dir")
        .resolve("exact.sfz")
        .expect("a line of exactly the cap is accepted");

    let one_less = ParseLimits {
        max_line_bytes: body.len() - 1,
        ..ParseLimits::default()
    };
    let error = IncludeResolver::new(root, one_less)
        .expect("base dir")
        .resolve("exact.sfz")
        .expect_err("one byte past the cap");
    assert!(
        matches!(
            error,
            SfzError::LineTooLong { len, limit, .. }
                if len == body.len() && limit == body.len() - 1
        ),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn include_error_line_numbers_are_one_based() {
    // 沙箱错误必须指向**文件里的 1-based 行号**：第 2 行的坏 include 报 2，不是 1。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(
        root,
        "main.sfz",
        "<region>sample=a.wav\n#include \"missing.sfz\"\n",
    );
    let error = resolve_and_parse(root, "main.sfz").expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeNotFound { line: 2, .. }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn a_segment_after_an_include_keeps_the_right_first_line() {
    // `SfzSource::first_line` 必须继续数下去：include 之后的 region 行号 = 文件行号；
    // **第二处** include 之前的片段也要拿它自己的起始行（两处 push 分支都要对）。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "inc.sfz", "<region>sample=inc.wav\n");
    write_file(
        root,
        "main.sfz",
        "#include \"inc.sfz\"\n<region>sample=second.wav\n#include \"inc.sfz\"\n<region>sample=fourth.wav\n",
    );
    let instrument = resolve_and_parse(root, "main.sfz").expect("resolves");
    let lines: Vec<usize> = instrument
        .regions()
        .iter()
        .map(|region| region.source_line)
        .collect();
    assert_eq!(lines, vec![1, 2, 1, 4]);
}

#[test]
fn the_default_include_depth_is_the_registered_constant() {
    // 缺省深度上限是 DoS 防线的一部分：这里钉的是它的**行为**读数
    // （16 个文件的链放行，17 个文件的链拒绝）。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    build_chain(root, 16);
    IncludeResolver::new(root, ParseLimits::default())
        .expect("base dir")
        .resolve("f0.sfz")
        .expect("the default depth admits 16 files");

    let deep = TempDir::new("sfz");
    let deep_root = deep.path();
    build_chain(deep_root, 17);
    let error = IncludeResolver::new(deep_root, ParseLimits::default())
        .expect("base dir")
        .resolve("f0.sfz")
        .expect_err("the default depth rejects 17 files");
    assert!(
        matches!(error, SfzError::IncludeDepthExceeded { limit: 16 }),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn a_wildcard_glob_does_not_descend_into_a_symlinked_directory() {
    // 通配符的「非末段」目录展开与递归 glob 是**两处**独立的 `is_symlink` 过滤，
    // 两处都要钉住：`kit/*/b.sfz` 不得穿过 `kit/linked` 这个链接目录。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "elsewhere/b.sfz", "<region>sample=through_link.wav\n");
    fs::create_dir_all(root.join("kit")).expect("create kit dir");
    std::os::unix::fs::symlink(root.join("elsewhere"), root.join("kit/linked"))
        .expect("create dir symlink");
    write_file(root, "main.sfz", "#include \"kit/*/b.sfz\"\n");
    let error = resolve_and_parse(root, "main.sfz").expect_err("the link must not match");
    assert!(
        matches!(error, SfzError::IncludeNoMatch { .. }),
        "unexpected verdict: {error:?}"
    );
}

// ---------------------------------------------------------------------------
// 第七批：此前「测试 0 引用」的 include 诊断臂与 `base_dir()`
// ---------------------------------------------------------------------------

#[test]
fn an_unterminated_include_string_is_reported() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "main.sfz", "#include \"inc.sfz\n");
    let error = IncludeResolver::new(root, ParseLimits::default())
        .expect("base dir")
        .resolve("main.sfz")
        .expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeUnterminated { line: 1 }),
        "unexpected verdict: {error:?}"
    );
    assert_eq!(error.to_string(), "line 1: unterminated #include string");
}

#[test]
fn an_empty_include_path_is_reported() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "main.sfz", "#include \"\"\n");
    let error = IncludeResolver::new(root, ParseLimits::default())
        .expect("base dir")
        .resolve("main.sfz")
        .expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeEmptyPath { line: 1 }),
        "unexpected verdict: {error:?}"
    );
    assert_eq!(error.to_string(), "line 1: #include path is empty");
}

#[test]
fn an_include_path_with_a_nul_byte_is_reported() {
    // 词法层拒绝 NUL（见 `check_relative` 的文档）；这也是 `IncludeInvalidPath` 的唯一入口。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "main.sfz", "#include \"a\0b.sfz\"\n");
    let error = IncludeResolver::new(root, ParseLimits::default())
        .expect("base dir")
        .resolve("main.sfz")
        .expect_err("must reject");
    assert!(
        matches!(error, SfzError::IncludeInvalidPath { line: 1, .. }),
        "unexpected verdict: {error:?}"
    );
    assert!(
        error.to_string().contains("is not a valid filesystem path"),
        "unexpected message: {error}"
    );
}

#[test]
fn a_base_that_is_not_a_directory_is_an_io_error() {
    // `IncludeResolver::new` 的第二个失败口：规范化成功但不是目录。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "plain.sfz", "<region>sample=a.wav\n");
    let error = IncludeResolver::new(root.join("plain.sfz"), ParseLimits::default())
        .expect_err("a file is not a base directory");
    assert!(
        matches!(error, SfzError::Io { .. }),
        "unexpected verdict: {error:?}"
    );
    assert!(
        error
            .to_string()
            .contains("include base is not a directory"),
        "unexpected message: {error}"
    );
}

#[test]
fn base_dir_reports_the_canonical_sandbox_root() {
    // `base_dir()` 是给调用方做展示 / 诊断的公开访问器：必须是**规范化后**的根。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    let resolver = IncludeResolver::new(root, ParseLimits::default()).expect("base dir");
    assert_eq!(
        resolver.base_dir(),
        fs::canonicalize(root).expect("canonical root")
    );
    // 通过 `..` 进根的等价写法必须归一到同一个根。
    fs::create_dir_all(root.join("sub")).expect("create sub dir");
    let via_parent =
        IncludeResolver::new(root.join("sub/.."), ParseLimits::default()).expect("base dir via ..");
    assert_eq!(via_parent.base_dir(), resolver.base_dir());
}

// ---------------------------------------------------------------------------
// 第八批：glob 深度的静默截断、6 个 include 侧限额字段、路径载荷的尺子
// ---------------------------------------------------------------------------

#[test]
fn the_glob_depth_cap_truncates_silently() {
    // `max_glob_depth` 与 `max_include_depth` 是**两种口径**：
    // - glob 深度超限 ⇒ 直接 `Ok(())` 停止下探（没有任何「深度超限」错误变体）；
    //   于是调用方看到的只是「没有匹配」。
    // - include 深度超限 ⇒ 明确 `Err(IncludeDepthExceeded)`。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "kit/sub/deep.sfz", "<region>sample=deep.wav\n");
    write_file(root, "main.sfz", "#include \"kit/**/*.sfz\"\n");
    let found = resolve_and_parse(root, "main.sfz").expect("the default depth finds it");
    assert_eq!(found.regions()[0].sample, "deep.wav");

    let limits = ParseLimits {
        max_glob_depth: 1,
        ..ParseLimits::default()
    };
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("main.sfz")
        .expect_err("a truncated walk leaves no match");
    assert!(
        matches!(error, SfzError::IncludeNoMatch { .. }),
        "the truncation is silent, so the verdict is `IncludeNoMatch`: {error:?}"
    );
}

#[test]
fn every_include_side_quota_field_is_wired_to_a_reachable_check() {
    // 与解析侧同一条纪律：把 6 个 include / glob 字段**单独**收回一个小值，
    // 每个字段自己的检查点都必须有可观测后果（唯一的例外是 glob 深度 —— 那是静默截断，
    // 由 `the_glob_depth_cap_truncates_silently` 单独钉住）。
    let full = ParseLimits::unlimited();

    // max_source_bytes
    let dir = TempDir::new("sfz");
    let root = dir.path();
    let body = "<region>sample=a.wav\n";
    write_file(root, "one.sfz", body);
    IncludeResolver::new(root, full)
        .expect("base dir")
        .resolve("one.sfz")
        .expect("unlimited accepts the file");
    let error = IncludeResolver::new(
        root,
        ParseLimits {
            max_source_bytes: 10,
            ..full
        },
    )
    .expect("base dir")
    .resolve("one.sfz")
    .expect_err("source byte cap");
    assert!(
        matches!(error, SfzError::SourceTooLarge { len, limit, .. } if len == body.len() && limit == 10),
        "{error:?}"
    );

    // max_include_depth（入口文件自身在深度 0）
    write_file(root, "leaf.sfz", body);
    write_file(root, "top.sfz", "#include \"leaf.sfz\"\n");
    IncludeResolver::new(root, full)
        .expect("base dir")
        .resolve("top.sfz")
        .expect("unlimited accepts one nesting level");
    let error = IncludeResolver::new(
        root,
        ParseLimits {
            max_include_depth: 0,
            ..full
        },
    )
    .expect("base dir")
    .resolve("top.sfz")
    .expect_err("depth cap");
    assert!(
        matches!(error, SfzError::IncludeDepthExceeded { limit: 0 }),
        "{error:?}"
    );

    // max_include_files（含入口文件）
    IncludeResolver::new(root, full)
        .expect("base dir")
        .resolve("top.sfz")
        .expect("unlimited accepts two files");
    let error = IncludeResolver::new(
        root,
        ParseLimits {
            max_include_files: 1,
            ..full
        },
    )
    .expect("base dir")
    .resolve("top.sfz")
    .expect_err("file count cap");
    assert!(
        matches!(error, SfzError::IncludeCountExceeded { limit: 1 }),
        "{error:?}"
    );

    // max_glob_matches
    write_file(root, "parts/a.sfz", body);
    write_file(root, "parts/b.sfz", body);
    write_file(root, "glob.sfz", "#include \"parts/*.sfz\"\n");
    IncludeResolver::new(root, full)
        .expect("base dir")
        .resolve("glob.sfz")
        .expect("unlimited accepts two matches");
    let error = IncludeResolver::new(
        root,
        ParseLimits {
            max_glob_matches: 1,
            ..full
        },
    )
    .expect("base dir")
    .resolve("glob.sfz")
    .expect_err("match cap");
    assert!(
        matches!(error, SfzError::GlobMatchesExceeded { limit: 1, .. }),
        "{error:?}"
    );

    // max_glob_scanned（`read_dir` 迭代次数）
    let error = IncludeResolver::new(
        root,
        ParseLimits {
            max_glob_scanned: 1,
            ..full
        },
    )
    .expect("base dir")
    .resolve("glob.sfz")
    .expect_err("scan cap");
    assert!(
        matches!(error, SfzError::GlobScanExceeded { limit: 1, .. }),
        "{error:?}"
    );

    // max_glob_depth（静默截断 ⇒ 仍然是「无匹配」）
    write_file(root, "kit/sub/deep.sfz", body);
    write_file(root, "deep.sfz", "#include \"kit/**/*.sfz\"\n");
    IncludeResolver::new(root, full)
        .expect("base dir")
        .resolve("deep.sfz")
        .expect("unlimited walks to the bottom");
    let error = IncludeResolver::new(
        root,
        ParseLimits {
            max_glob_depth: 1,
            ..full
        },
    )
    .expect("base dir")
    .resolve("deep.sfz")
    .expect_err("truncated walk");
    assert!(
        matches!(error, SfzError::IncludeNoMatch { .. }),
        "the glob depth cap is the one silent cap: {error:?}"
    );
}

#[test]
fn source_and_payload_paths_are_the_display_relative_path() {
    // `SfzSource::path`（此前全仓无读取点 ⇒ 本 crate 内是**只写字段**）与错误载荷里的
    // `path` 是**同一把尺子**：相对基准目录、`/` 分隔的展示路径 —— 不是绝对路径、
    // 也不是文件名。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "inc.sfz", "<region>sample=inc.wav\n");
    write_file(
        root,
        "main.sfz",
        "<region>sample=main.wav\n#include \"inc.sfz\"\n#include \"parts/*.sfz\"\n<region>sample=tail.wav\n",
    );
    let limits = ParseLimits::default();
    let sources = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("main.sfz")
        .expect("resolves");
    let paths: Vec<&str> = sources.iter().map(|source| source.path.as_str()).collect();
    // 两处片段 push（include 之前的文本、循环结束后的尾段）都带同一把尺子的路径。
    assert_eq!(
        paths,
        vec!["main.sfz", "inc.sfz", "parts/a.sfz", "main.sfz"]
    );

    // `IncludeNotFound`：载荷是请求的相对路径。
    write_file(root, "bad.sfz", "#include \"missing/inc.sfz\"\n");
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("bad.sfz")
        .expect_err("must reject");
    assert!(
        matches!(&error, SfzError::IncludeNotFound { line: 1, path } if path == "missing/inc.sfz"),
        "unexpected verdict: {error:?}"
    );

    // `SourceTooLarge`：同一个相对路径 + 实际字节数。
    write_file(root, "big.sfz", "<region>sample=a.wav\n");
    let tight = ParseLimits {
        max_source_bytes: 10,
        ..ParseLimits::default()
    };
    let error = IncludeResolver::new(root, tight)
        .expect("base dir")
        .resolve("big.sfz")
        .expect_err("must reject");
    assert!(
        matches!(&error, SfzError::SourceTooLarge { path, len, limit } if path == "big.sfz" && *len == 21 && *limit == 10),
        "unexpected verdict: {error:?}"
    );

    // `NotUtf8`：同一个相对路径。
    fs::write(root.join("bin.sfz"), [0xff, 0xfe, 0x00]).expect("write non-utf8 bytes");
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("bin.sfz")
        .expect_err("must reject");
    assert!(
        matches!(&error, SfzError::NotUtf8 { path } if path == "bin.sfz"),
        "unexpected verdict: {error:?}"
    );

    // `IncludeCycle`：载荷是**闭合环**的那个相对路径。
    write_file(root, "a.sfz", "#include \"b.sfz\"\n");
    write_file(root, "b.sfz", "#include \"a.sfz\"\n");
    let error = IncludeResolver::new(root, limits)
        .expect("base dir")
        .resolve("a.sfz")
        .expect_err("must reject");
    assert!(
        matches!(&error, SfzError::IncludeCycle { path } if path == "a.sfz"),
        "unexpected verdict: {error:?}"
    );
}

#[test]
fn every_include_side_quota_at_zero_fires_its_own_check() {
    // 与解析侧同一条纪律：6 个 include / glob 字段调到**极小（0）**时的可观测后果。
    let dir = TempDir::new("sfz");
    let root = dir.path();
    write_file(root, "one.sfz", "x"); // 1 字节
    write_file(root, "leaf.sfz", "<region>sample=a.wav\n");
    write_file(root, "top.sfz", "#include \"leaf.sfz\"\n");
    write_file(root, "parts/a.sfz", "<region>sample=a.wav\n");
    write_file(root, "glob.sfz", "#include \"parts/*.sfz\"\n");
    write_file(root, "kit/sub/deep.sfz", "<region>sample=deep.wav\n");
    write_file(root, "deep.sfz", "#include \"kit/**/*.sfz\"\n");

    let cases: &[(&str, ParseLimits, &str)] = &[
        (
            "one.sfz",
            ParseLimits {
                max_source_bytes: 0,
                ..ParseLimits::default()
            },
            "source bytes",
        ),
        (
            "top.sfz",
            ParseLimits {
                max_include_depth: 0,
                ..ParseLimits::default()
            },
            "depth",
        ),
        (
            "top.sfz",
            ParseLimits {
                max_include_files: 0,
                ..ParseLimits::default()
            },
            "files",
        ),
        (
            "glob.sfz",
            ParseLimits {
                max_glob_matches: 0,
                ..ParseLimits::default()
            },
            "matches",
        ),
        (
            "glob.sfz",
            ParseLimits {
                max_glob_scanned: 0,
                ..ParseLimits::default()
            },
            "scanned",
        ),
        (
            "deep.sfz",
            ParseLimits {
                max_glob_depth: 0,
                ..ParseLimits::default()
            },
            "glob depth",
        ),
    ];
    for (entry, limits, label) in cases {
        let error = IncludeResolver::new(root, *limits)
            .expect("base dir")
            .resolve(entry)
            .expect_err("a zero quota must be observable");
        let matches_variant = match *label {
            "source bytes" => matches!(error, SfzError::SourceTooLarge { limit: 0, .. }),
            "depth" => matches!(error, SfzError::IncludeDepthExceeded { limit: 0 }),
            "files" => matches!(error, SfzError::IncludeCountExceeded { limit: 0 }),
            "matches" => matches!(error, SfzError::GlobMatchesExceeded { limit: 0, .. }),
            "scanned" => matches!(error, SfzError::GlobScanExceeded { limit: 0, .. }),
            // 唯一「静默」的限额：glob 深度 0 ⇒ 只剩基目录一层 ⇒ 变成无匹配。
            _ => matches!(error, SfzError::IncludeNoMatch { .. }),
        };
        assert!(matches_variant, "{label}: unexpected verdict {error:?}");
    }
}
