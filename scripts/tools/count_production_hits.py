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
    print("[ok] 自检通过：生产命中可见、测试夹具已排除")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) >= 2 and argv[1] == "--self-test":
        return self_test()
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
