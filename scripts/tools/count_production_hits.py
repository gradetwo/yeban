#!/usr/bin/env python3
"""数"生产命中" —— 剥离注释与 `#[cfg(test)]` 整块后统计（R515/R519/R522 的教训固化）。

⭐ 为什么需要它: 本会话"粗数被污染"已四次 ——
  ① 模块文档里的字面量（R515）② 注释/echo 里的同名子串（R515）
  ③ `#[cfg(test)]` 里的夹具（R519）④ 只取第一个 `cfg(test)` 边界而漏判（R522）。

用法:
    python3 scripts/tools/count_production_hits.py <needle> <file> [<file>...]
    python3 scripts/tools/count_production_hits.py --self-test

口径（⭐ 报数时必须声明排除了什么）:
  · 排除**注释**（`//` 与 `/* */`，含 `///` 文档注释）
  · 排除**整个 `#[cfg(test)]` 块**（按花括号配对，支持多块）
  · ⚠ 不解析字符串字面量内的 `/*`（极罕见）；命中数只作**候选**、需人工核。
"""
from __future__ import annotations

import pathlib
import re
import sys


def strip_comments(text: str) -> str:
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def strip_cfg_test_blocks(text: str) -> str:
    """把每个 `#[cfg(test)]` 之后**紧跟的**那个块整段去掉（花括号配对，支持多块）。"""
    out = text
    while True:
        m = re.search(r"#\[cfg\(test\)\]", out)
        if not m:
            return out
        # 从属性之后找第一个 '{'，再配平到它的 '}'
        start = out.find("{", m.end())
        if start < 0:
            return out[: m.start()]
        depth, i, n = 0, start, len(out)
        while i < n:
            c = out[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        out = out[: m.start()] + out[i + 1 :]


def count(needle: str, path: str) -> int:
    src = open(path, encoding="utf-8").read()
    body = strip_cfg_test_blocks(strip_comments(src))
    return len(re.findall(r"\b" + re.escape(needle) + r"\b", body))


def non_production_rs_paths(crate_dir: str) -> set[str]:
    """从 `<crate_dir>/Cargo.toml` 读出**不是交付产物**的 `.rs` 路径。

    ⭐ 为什么需要它（R528 的教训）：`src/` 下的文件**不都是生产** ——
      · `[[test]]` / `[[bench]]` 显式声明的 target（例：`yeban-app` 的 `src/test_port_adapter.rs`）
      · `[[example]]`
      · `build.rs`（构建脚本，不随产物运行）
    ⚠ 只靠"在 src 里且不在 cfg(test) 里"会把它们算成生产 ⇒ 读数偏高。
    """
    cargo = pathlib.Path(crate_dir) / "Cargo.toml"
    if not cargo.exists():
        return set()
    try:
        import tomllib  # py3.11+
        data = tomllib.loads(cargo.read_text(encoding="utf-8"))
    except Exception:
        # 退化路径：正则粗读（够用，因为只要 path = "…" 那些行）
        data = {}
    out: set[str] = set()
    for kind in ("test", "bench", "example"):
        for item in data.get(kind, []) or []:
            p = (item or {}).get("path")
            if p:
                out.add(str((pathlib.Path(crate_dir) / p).resolve()))
    out.add(str((pathlib.Path(crate_dir) / "build.rs").resolve()))
    return out


def production_rs_files(root: str) -> list[pathlib.Path]:
    """枚举**生产** `.rs`：跳过 `/tests/`、`build.rs`、以及 `Cargo.toml` 声明的 test/bench/example target。"""
    skip: set[str] = set()
    for cargo in pathlib.Path(root).rglob("Cargo.toml"):
        skip |= non_production_rs_paths(str(cargo.parent))
    files = []
    for f in sorted(pathlib.Path(root).rglob("*.rs")):
        if "/tests/" in str(f) or f.name == "build.rs":
            continue
        if str(f.resolve()) in skip:
            continue
        files.append(f)
    return files


def field_stats(root: str) -> tuple[int, int, int]:
    """数 `pub` 字段：① 出现次数（含测试）② 去重后的字段**名**数 ③ 排除 `#[cfg(test)]` 的出现次数。

    ⭐ 为什么要三个数并列（R524 的教训）：我一度把"去重名数"（132）当成"字段数"、
    又误以为它和另一个读数（179）的差来自"字段式样不同" —— 其实**两个数各自都对**，
    差的是**同名字段**。⭐ 并列显示后这类混淆就不会再发生。
    """
    pat = re.compile(r"^\s*pub ([a-z_][a-z0-9_]*): ", re.M)  # ⚠ 必须 re.M（R524 的静默 0 就是漏了它）
    occurrences = 0
    names: set[str] = set()
    prod = 0
    for f in production_rs_files(root):
        src = f.read_text(encoding="utf-8")
        for name in pat.findall(src):
            occurrences += 1
            names.add(name)
        prod += len(pat.findall(strip_cfg_test_blocks(strip_comments(src))))
    return occurrences, len(names), prod


def self_test() -> int:
    """正对照 + 已知答案：能看见生产命中、看不见测试夹具。"""
    cases = [
        # (needle, file, 期望)
        ("sync_all", "crates/yeban-model/src/local_config.rs", ">=2"),
        ("fs::write", "crates/yeban-mcp/src/domain/store.rs", "==0"),
        ("fs::write", "crates/yeban-mcp/src/security.rs", "==0"),
    ]
    bad = 0
    for needle, path, want in cases:
        got = count(needle, path)
        ok = (got >= 2) if want == ">=2" else (got == 0)
        print(f"[{'ok' if ok else 'FAIL'}] {path}  {needle!r} => {got}  (期望 {want})")
        bad += 0 if ok else 1
    if bad:
        print(f"[FAIL] 自检 {bad} 项不符 —— 扫描器或口径有问题")
        return 1
    # ⭐ 字段模式的自检：必须能数到非零、且"去重名数"严格小于"出现次数"
    #（二者相等 ⇒ 要么没数到、要么去重没生效）。
    occ, distinct, prod = field_stats("crates/yeban-model/src")
    ok = occ > 0 and 0 < distinct < occ and prod > 0
    print(
        f"[{'ok' if ok else 'FAIL'}] 字段模式自检: 出现={occ} 去重名={distinct} "
        f"排除测试={prod}  (要求 出现>0、0<去重名<出现、排除测试>0)"
    )
    if not ok:
        print("[FAIL] 字段模式自检不符")
        return 1
    # ⭐ 排除规则自检（R528）：`[[test]]` target 必须**不在**生产清单里、而普通 src 文件必须在。
    prod_names = {f.name for f in production_rs_files("crates/yeban-app")}
    ok2 = "test_port_adapter.rs" not in prod_names and "lib.rs" in prod_names
    print(
        f"[{'ok' if ok2 else 'FAIL'}] 排除规则自检: yeban-app 生产文件 {len(prod_names)} 个；"
        f"test_port_adapter.rs 在否={'test_port_adapter.rs' in prod_names}（须 False）、"
        f"local_config.rs 在否={'local_config.rs' in prod_names}（须 True）"
    )
    if not ok2:
        print("[FAIL] 排除规则自检不符")
        return 1
    print("[ok] 自检通过：生产命中可见、测试夹具已排除、字段三读数一致、[[test]] target 已排除")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) >= 2 and argv[1] == "--self-test":
        return self_test()
    if len(argv) >= 3 and argv[1] == "--fields":
        occ, distinct, prod = field_stats(argv[2])
        print(f"{argv[2]}: 字段出现次数={occ}  去重后字段名数={distinct}  排除 cfg(test) 后={prod}")
        print("⚠ 引用时请说明用的是哪一个口径（三者含义不同，R524 的混淆就出在这里）")
        return 0
    if len(argv) >= 3 and argv[1] == "--list-production":
        files = production_rs_files(argv[2])
        for f in files:
            print(f)
        print(f"共 {len(files)} 个生产 .rs（已排除 /tests/、build.rs、Cargo.toml 声明的 test/bench/example）")
        return 0
    if len(argv) < 3:
        print(__doc__)
        return 2
    needle, files = argv[1], argv[2:]
    total = 0
    for f in files:
        c = count(needle, f)
        total += c
        print(f"{f}: {c}")
    print(f"合计（已排除注释与 #[cfg(test)] 整块）: {total}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
