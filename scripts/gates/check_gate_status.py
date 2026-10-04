#!/usr/bin/env python3
"""校验 `docs/ledger/gate-status.md` 这张"什么算绿"的表本身没有腐烂。

为什么需要一条**元判据**: 那张表是人读"现在到底什么算绿"的唯一去处。
它一旦与事实脱节, 后果不是"少一条判据", 而是**所有人基于错误的现状做决定**
（本仓库已经踩过多次同族: L12 门禁空跑、D25 契约空转、`required-features` 绿着跳过）。

本脚本只做**机械可判定**的部分（语义是否正确仍要人看）:
1. `MUST-GATE-001..015` 与 `BASELINE-001..006` **各出现且仅出现一次**（漏一条 = 有门禁没人管）;
2. 每行的状态只能是 `已接线` / `部分` / `PENDING` 三者之一;
3. **非 PENDING 的行必须给出证据**（测试名 / run id / 命令 / 文件）—— 空证据是"口头绿";
4. `PENDING` 的行必须写清**为什么**还 PENDING（列里不能只有状态词）。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TABLE = REPO / "docs/ledger/gate-status.md"

MUST_GATES = [f"MUST-GATE-{index:03d}" for index in range(1, 16)]
BASELINES = [f"BASELINE-{index:03d}" for index in range(1, 7)]
VALID_STATUS = ("已接线", "部分", "PENDING")

#: 门禁的"要求"列（`--summary` 用；由 `main` 从表里填充）。
REQUIREMENTS: dict[str, str] = {}

#: 证据列里必须出现这类可复跑的东西之一, 否则不算证据。
EVIDENCE_HINTS = ("run ", "cargo ", "run-gates", "scripts/", "crates/", "docs/", "policy_check", "validate_schemas")


def emit_summary(rows: dict[str, tuple[str, str]]) -> None:
    """把表格渲染成 markdown, 供 CI 的 job summary 使用。

    为什么要有这个: `gates-manual.yml` 的 `inventory` 作业原先**手抄**了一份门禁清单,
    它与本表逐渐分叉(手抄那份到第 6 轮还在说 MUST-GATE-001/002/003… 是 PENDING,
    而本表早已是"已接线/部分")。**同一事实出现两处, 必然有一处是错的。**
    现在 inventory 直接调用本脚本生成, 手抄那份不再存在。
    """
    print("### 门禁清单（**自动取自** `docs/ledger/gate-status.md`，唯一事实源）")
    print()
    print("| 门禁 | 内容 | 状态 |")
    print("| :--- | :--- | :--- |")
    for ident, (status, _) in rows.items():
        requirement = REQUIREMENTS.get(ident, "")
        print(f"| `{ident}` | {requirement} | {status} |")
    print()
    print("> 逐条证据（可复跑的 run id / 命令）见 `docs/ledger/gate-status.md`；")
    print("> 本表由 `scripts/gates/check_gate_status.py --summary` 生成，**不再手抄**。")


def parse_rows(text: str) -> tuple[dict[str, tuple[str, str]], dict[str, str]]:
    """从状态表文本里解析出 `{编号: (状态, 证据)}` 与 `{编号: 要求}`。"""
    rows: dict[str, tuple[str, str]] = {}
    requirements: dict[str, str] = {}
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 4:
            continue
        ident = cells[0].strip("`")
        if re.fullmatch(r"(MUST-GATE|BASELINE)-[0-9]{3}", ident):
            rows[ident] = (cells[2], cells[3])
            requirements.setdefault(ident, cells[1][:60])
    return rows, requirements


def main() -> int:
    if "--summary" in sys.argv:
        # 供 CI 的 job summary 使用: **直接取自唯一事实源**, 不再手抄。
        rows, requirements = parse_rows(TABLE.read_text(encoding="utf-8"))
        REQUIREMENTS.update(requirements)
        emit_summary(rows)
        return 0

    if not TABLE.is_file():
        print(f"缺少门禁状态表: {TABLE}", file=sys.stderr)
        return 1
    text = TABLE.read_text(encoding="utf-8")
    problems: list[str] = []

    rows: dict[str, tuple[str, str]] = {}
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 4:
            continue
        ident = cells[0].strip("`")
        if re.fullmatch(r"(MUST-GATE|BASELINE)-[0-9]{3}", ident):
            if ident in rows:
                problems.append(f"{ident} 在表里出现了不止一次")
            rows[ident] = (cells[2], cells[3])

    for ident in MUST_GATES + BASELINES:
        if ident not in rows:
            problems.append(f"{ident} 不在表里 —— 有门禁没人管")
            continue
        status, evidence = rows[ident]
        matched = [candidate for candidate in VALID_STATUS if candidate in status]
        if len(matched) != 1:
            problems.append(f"{ident}: 状态 `{status}` 不是 {VALID_STATUS} 之一(或含多个)")
            continue
        if matched[0] == "PENDING":
            if len(evidence) < 10:
                problems.append(f"{ident}: PENDING 但没写清原因(证据列太短)")
        else:
            if not any(hint in evidence for hint in EVIDENCE_HINTS):
                problems.append(f"{ident}: 状态 `{matched[0]}` 但没有可复跑的证据(测试名/run id/命令/文件)")

    if problems:
        print("门禁状态表校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1
    print(f"[ok] gate-status.md: {len(MUST_GATES)} 条 MUST-GATE + {len(BASELINES)} 条 BASELINE 均已登记且带证据/原因")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
