#!/usr/bin/env python3
"""校验 `docs/ledger/human-decisions.md`：**待人类裁决的清单不能悄悄漏项或空转**。

为什么需要这条元判据: 那份清单是人类唯一的决策入口。它有两种典型的腐烂方式:
1. **漏项**: ADR 里新写了一条 `Proposed` 裁决, 却没进清单 ⇒ 人类永远看不到它, 而 Agent 照旧执行;
2. **空转**: 某行只写"请裁决", 没写**选项/建议/不决定的后果** ⇒ 人类无法判断, 只能反问,
   于是一个来回变成三个来回。

本脚本只做**机械可判定**的部分(语义仍要人看):
- ADR 里每个**标了 `Proposed`/`待人类`/`需人类` 的 `D<n>` 段落**, 必须在清单里出现;
- 清单每行必须有 6 列(ID/问题/选项/建议/后果)且 `HD-nn` 编号唯一;
- 清单必须写明"当前处置"(即: 不等裁决也能继续推进, 不构成单点阻塞)。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
ADR = next(REPO.glob("docs/adr/ADR-0001-*.md"), None)
DECISIONS = REPO / "docs/ledger/human-decisions.md"

#: 出现这些词就认为"这条裁决在等人类"。
HUMAN_MARKERS = ("Proposed", "待人类", "需人类", "待追认")


def adr_decisions_awaiting_human(text: str) -> set[str]:
    """从 ADR 里抽出"标了人类标记"的裁决编号。"""
    awaiting: set[str] = set()
    # 按**任意标题**切段。第一版只按 `D<n>` 标题切, 于是"最后一条裁决"会把文件尾部
    # 那一整节（含 `## 待人类批准/补充`）粘进自己的段里, 从而被误判为"在等人类" ——
    # 那是解析器的假阳, 不是 ADR 的问题。（由 check_decisions.py 自己第一次运行暴露。）
    sections = re.split(r"\n(?=#{1,4} )", text)
    for section in sections:
        heading = re.match(r"#{1,4} (D\d+)", section)
        if not heading:
            continue
        ident = heading.group(1)
        # 只看**本段自己的正文**（去掉标题行）, 避免把别处顺带提到的词算进来
        body = section.split("\n", 1)[1] if "\n" in section else ""
        if any(marker in body[:1200] for marker in HUMAN_MARKERS):
            awaiting.add(ident)
    return awaiting


def main() -> int:
    if ADR is None:
        print("找不到 ADR 文件", file=sys.stderr)
        return 1
    if not DECISIONS.is_file():
        print(f"缺少待裁决清单: {DECISIONS}", file=sys.stderr)
        return 1

    adr_text = ADR.read_text(encoding="utf-8")
    decisions_text = DECISIONS.read_text(encoding="utf-8")
    problems: list[str] = []

    awaiting = adr_decisions_awaiting_human(adr_text)
    for ident in sorted(awaiting, key=lambda s: int(s[1:])):
        if not re.search(rf"\*\*{ident}\*\*", decisions_text):
            problems.append(
                f"ADR 里的 {ident} 标了人类标记, 但 {DECISIONS.name} 里没有对应行 "
                f"—— 人类看不到它, 而 Agent 会照旧执行"
            )

    rows = 0
    seen_ids: set[str] = set()
    for line in decisions_text.splitlines():
        if not line.startswith("| `HD-"):
            continue
        rows += 1
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        ident = cells[0].strip("`")
        if ident in seen_ids:
            problems.append(f"{ident} 出现了不止一次")
        seen_ids.add(ident)
        if len(cells) < 5:
            problems.append(f"{ident}: 列数不足(需要 ID/问题/选项/建议/后果), 实际 {len(cells)}")
            continue
        _, question, options, advice, consequence = cells[:5]
        if len(question) < 8:
            problems.append(f"{ident}: 问题写得太短")
        if len(options) < 8 or "A" not in options:
            problems.append(f"{ident}: 选项没有列出可选方案(至少 A/B)")
        if len(advice) < 2:
            problems.append(f"{ident}: 没给建议(人类需要知道 Agent 倾向哪个)")
        if len(consequence) < 8:
            problems.append(f"{ident}: 没写'不决定的后果/当前处置' ⇒ 无法判断能否先不动")

    if rows == 0:
        problems.append("清单里一行都没有")
    if "不构成单点阻塞" not in decisions_text and "单点阻塞" not in decisions_text:
        problems.append("清单没有声明'不等裁决也能继续推进'(否则它本身就是单点阻塞)")

    # 已裁决的行必须带**日期** —— 否则'已裁决'只是一句话, 无法追溯是哪一轮、依谁的授权落定的。
    decided = 0
    for line in decisions_text.splitlines():
        if not line.startswith("| `HD-"):
            continue
        if "✅" in line:
            decided += 1
            if not re.search(r"\d{4}-\d{2}-\d{2}", line):
                problems.append(
                    f"{line.split('|')[1].strip()}: 标了已裁决但没有日期"
                )

    if problems:
        print("待裁决清单校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1
    summary = (
        f"[ok] human-decisions.md: {rows} 项"
        f"(其中 {decided} 项已裁决); "
        f"ADR 里 {len(awaiting)} 条人类标记的裁决全部在册"
    )
    print(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
