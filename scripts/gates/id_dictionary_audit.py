#!/usr/bin/env python3
"""审计 `AGENTS.md` §4 的**规范 ID 命名字典**与四份规范的实际 ID 是否一致。

为什么需要它: `AGENTS.md` §4 是 Agent 判断"**有哪些 ID 可以用**"的权威索引 ——
而它自己承认过一处漂移的先例（`MODEL-AST-006` 在规范里缺号, 不得凭空发明）。
索引与事实不一致有两种代价:
① 索引**少**了某个族 ⇒ Agent 以为不能用, 于是**发明**一个新编号;
② 索引**多**了或范围写错 ⇒ Agent 引用一个不存在的 ID（`spec_id_audit.py --check` 会在提交时拦下, 但那时已白写）。
本脚本把两者都**在写代码之前**摆在桌面上。

用法: `python3 scripts/gates/id_dictionary_audit.py`（只读）
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
AGENTS = REPO / "AGENTS.md"
SPECS = sorted(REPO.glob("docs/YEBAN_*.md"))

# ⚠ 后缀限定为 `0xx`（三位且以 0 开头）: 规范 ID 的编号都在 001–099 区间。
# 不加这条会命中真实世界的词 —— 实测第一版把 `SHA-256`（哈希算法）与 `TR-808`（鼓机）
# 当成了"规范 ID", 报出两个假阳。
ID_RE = re.compile(r"\b([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*)-(0\d{2})\b")
#: 字典行的形态: `- \`ARCH-RT-*\`：…（001–005）` 或 `- \`MODEL-AST-001..005, 007\`：…`
DICT_TOKENS_RE = re.compile(r"`([^`]+)`")


def family(ident: str) -> str:
    return ident.rsplit("-", 1)[0]


def spec_families() -> dict[str, list[int]]:
    out: dict[str, set[int]] = {}
    for path in SPECS:
        for match in ID_RE.finditer(path.read_text(encoding="utf-8")):
            out.setdefault(match.group(1), set()).add(int(match.group(2)))
    return {name: sorted(values) for name, values in sorted(out.items())}


def dictionary_families() -> dict[str, list[int] | None]:
    """字典里声明的族 -> 声明覆盖的编号（`None` = 只写了族名, 没写范围）。"""
    text = AGENTS.read_text(encoding="utf-8")
    section = text[text.index("## 4. 规范需求 ID 命名字典"):]
    section = section[: section.index("\n## 5.")] if "\n## 5." in section else section
    out: dict[str, list[int] | None] = {}
    for line in section.splitlines():
        if not line.startswith("- "):
            continue
        # ⚠ 一行里可能有**多个**反引号 token（如 `ARCH-PLUG-*` / `ARCH-REC-*` / `ARCH-EXT-*`）——
        # 第一版只取第一个, 于是把 REC/EXT/SYS 三族误报成"字典没提到"。
        for token in DICT_TOKENS_RE.findall(line):
            _register(out, token)
    return out


def _register(out: dict[str, list[int] | None], token: str) -> None:
    if True:
        token = token.strip().rstrip("，,：:。")
        # 形态 A: `FAMILY-*`（可写若干族, 用 ` / ` 分隔）
        if token.endswith("-*"):
            for name in token.split("/"):
                name = name.strip().rstrip("-*").strip()
                if name:
                    out.setdefault(name, None)
            return
        # 形态 B: 显式编号清单, 如 `MODEL-AST-001..005, 007`
        base = token.split()[0].rstrip(",")
        parsed = re.match(r"^([A-Z][A-Z0-9-]*?)-(\d{3})(?:\.\.(\d{3}))?$", base)
        if parsed:
            fam = parsed.group(1)
            start = int(parsed.group(2))
            end = int(parsed.group(3)) if parsed.group(3) else start
            numbers = list(range(start, end + 1))
            for extra in re.findall(r",\s*(\d{3})", token):
                numbers.append(int(extra))
            out.setdefault(fam, sorted(set(numbers)))


def main() -> int:
    specs = spec_families()
    dictionary = dictionary_families()
    problems: list[str] = []

    for fam, numbers in sorted(specs.items()):
        if fam not in dictionary:
            problems.append(
                f"规范里有 `{fam}`（{len(numbers)} 个编号: {numbers[0]:03d}–{numbers[-1]:03d}）"
                f"但 AGENTS.md §4 字典**没有提到这个族** ⇒ Agent 会以为不能用它"
            )
            continue
        claimed = dictionary[fam]
        if claimed is None:
            continue  # 只写了族名, 不算错
        if claimed != numbers:
            problems.append(
                f"`{fam}` 的编号不一致: 字典写 {claimed} vs 规范实际 {numbers}"
            )

    for fam in sorted(dictionary):
        if fam not in specs:
            problems.append(f"字典里有 `{fam}` 但四份规范里**一个该族的 ID 都没有**")

    if problems:
        print("ID 字典与规范不一致:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1
    print(f"[ok] id_dictionary_audit: 字典的 {len(dictionary)} 个族与规范的 {len(specs)} 个族一致")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
