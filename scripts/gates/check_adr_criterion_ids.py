#!/usr/bin/env python3
"""ADR-0077 判据 ID 门禁（效果总线清单 I01）。

判据：`docs/adr/ADR-0077-sfz-effect-bus.md` 里登记的每一个判据 ID，
都必须在 `crates/` 的 Rust 源码里至少出现一次（测试 doc 锚点、文件头
对照表或断言注释均可）。只活在 ADR 表里、代码里一次都没提的 ID，
就是「判据表挂着、代码里没有」——本门禁正是防这一类漂移
（作者在 r12 抓到的 SFX-11 即此型）。

ID 抽取口径：
- `SFX-<n>` 与 `S-bus-<n>`：全文抽取（这两族只作判据 ID 用）。
- `<大写字母><两位数字>`（A01、E04、F01、G01 型）：只从表格行抽取，
  避免正文里的同形词被误收。

退出码：全覆盖 0；有缺失 1（缺失清单打到 stdout，逐条一行）。
用法：`python3 scripts/gates/check_adr_criterion_ids.py [--root <仓库根>]`
缺省 root 为脚本所在仓库根。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ADR_REL = Path("docs/adr/ADR-0077-sfz-effect-bus.md")

FREE_ID = re.compile(r"\b(?:SFX-\d+|S-bus-\d+)\b")
TABLE_ID = re.compile(r"\b[AEFG]\d{2}\b")


def extract_ids(adr_text: str) -> list[str]:
    ids: set[str] = set(FREE_ID.findall(adr_text))
    for line in adr_text.splitlines():
        if line.lstrip().startswith("|"):
            ids.update(TABLE_ID.findall(line))
    return sorted(ids)


def covered_ids(root: Path, ids: list[str]) -> dict[str, list[str]]:
    """返回 {id: [命中的相对路径, ...]}，只收录有命中的 ID。"""
    hits: dict[str, list[str]] = {i: [] for i in ids}
    pats = {i: re.compile(r"(?<![A-Za-z0-9])" + re.escape(i) + r"(?![0-9])") for i in ids}
    for rs in sorted((root / "crates").rglob("*.rs")):
        text = rs.read_text(encoding="utf-8", errors="replace")
        for i, pat in pats.items():
            if pat.search(text):
                hits[i].append(str(rs.relative_to(root)))
    return {i: v for i, v in hits.items() if v}


def main(argv: list[str]) -> int:
    root = Path(__file__).resolve().parents[2]
    if "--root" in argv:
        root = Path(argv[argv.index("--root") + 1]).resolve()
    adr_path = root / ADR_REL
    if not adr_path.is_file():
        print(f"FAIL: ADR 文档不存在: {adr_path}")
        return 1
    ids = extract_ids(adr_path.read_text(encoding="utf-8"))
    if not ids:
        print("FAIL: ADR 里没有抽到任何判据 ID，抽取口径可能已失效")
        return 1
    hits = covered_ids(root, ids)
    missing = [i for i in ids if i not in hits]
    print(f"ADR-0077 判据 ID 共 {len(ids)} 个，crates/ 已覆盖 {len(hits)} 个")
    for i in ids:
        if i in hits:
            print(f"  OK      {i}  ({hits[i][0]})")
        else:
            print(f"  MISSING {i}")
    if missing:
        print(f"FAIL: {len(missing)} 个判据 ID 在 crates/ 里无锚点: {', '.join(missing)}")
        return 1
    print("PASS: 全部判据 ID 在 crates/ 里有锚点")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
