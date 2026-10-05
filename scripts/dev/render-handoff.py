#!/usr/bin/env python3
"""把两份台账渲染成一份**交接快照** —— 数字与清单都取自唯一事实源, 不手写。

为什么需要它: `docs/ledger/gate-status.md` 与 `phase-status.md` 是权威, 但它们很长;
交接者需要一个"现在到底什么算绿、还差什么、需要人类做什么"的单页。手写这种页面必然腐烂
（本仓库已有过台账与事实脱节的教训）, 所以这里**每次由脚本重新生成**。

用法: python3 scripts/dev/render-handoff.py            # 写入 docs/ledger/handoff-snapshot.md
"""
from __future__ import annotations

import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
OUT = pathlib.Path(
    os.environ.get("HANDOFF_OUT", str(REPO / "docs" / "ledger" / "handoff-snapshot.md"))
)


def rows(path: pathlib.Path) -> list[tuple[str, str, str]]:
    """抽出台账里 `| `ID` | ... | **状态** | ... |` 形式的 (id, 状态)。"""
    out: list[tuple[str, str, str]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith("| `"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) < 3:
            continue
        ident = cells[0].strip("`")
        status = ""
        for c in cells[1:]:
            # 状态格里常带括号说明（如 `**PENDING（机器已就绪…）**`）⇒ 必须按前缀判定,
            # 否则会漏行（第一版就这样把 21 条数成了 17 条）。
            bare = c.strip("* ").strip()
            for word in ("已完成", "已接线", "部分", "PENDING"):
                if bare.startswith(word):
                    status = word
                    break
            if status:
                break
        if status:
            reason = cells[-1] if len(cells) > 3 else ""
            reason = " ".join(reason.split())[:200]
            out.append((ident, status, reason))
    return out


def summarize(items: list[tuple[str, str, str]]) -> dict[str, int]:
    d: dict[str, int] = {}
    for _i, s, _r in items:
        d[s] = d.get(s, 0) + 1
    return d


def main() -> int:
    gates = rows(REPO / "docs" / "ledger" / "gate-status.md")
    phases = rows(REPO / "docs" / "ledger" / "phase-status.md")
    if not gates or not phases:
        print("台账读取为空 —— 渲染中止(不产出空快照)", file=sys.stderr)
        return 1
    g, p = summarize(gates), summarize(phases)
    lines = [
        "# 交接快照（**由 `scripts/dev/render-handoff.py` 生成，请勿手改**）",
        "",
        "数字与清单都直接取自 `gate-status.md` / `phase-status.md`；本页只做汇总，**不引入新事实**。",
        "",
        "## 门禁（`docs/ledger/gate-status.md`）",
        "",
        f"- 已接线 **{g.get('已接线', 0)}** · 部分 **{g.get('部分', 0)}** · PENDING **{g.get('PENDING', 0)}**"
        f"（共 {len(gates)} 条）",
        "",
        "## 阶段项（`docs/ledger/phase-status.md`）",
        "",
        f"- 已完成 **{p.get('已完成', 0)}** · 部分 **{p.get('部分', 0)}** · PENDING **{p.get('PENDING', 0)}**"
        f"（共 {len(phases)} 项）",
        "",
        "## 仍未闭环的门禁（逐条）",
        "",
    ]
    for ident, status, reason in gates:
        if status != "已接线":
            lines.append(f"- `{ident}` — **{status}**。理由：{reason}")
    lines += ["", "## 仍未闭环的阶段项（逐条）", ""]
    for ident, status, _reason in phases:
        if status != "已完成":
            lines.append(f"- `{ident}` — **{status}**")
    lines += [
        "",
        "## 待人类决策（未闭环的门禁里, 属于负责人裁决的那几条）",
        "",
        "逐条见上面「仍未闭环的门禁」。每条都写明选项。请负责人选一条。",
        "",
        "## 读这份快照的纪律",
        "",
        "- **判决只能来自 CI**（`ci.yml` 自动档 / `gates-manual.yml` 手动档），本页的数字**不是**判决。",
        "- 逐腿要看 `conclusion` **与 `steps` 数**：`steps=0` 的 success 是空心绿。",
        "- 每条「已完成/已接线」的背后应当有一条**可复跑**的命令或 run id；没有就回 `gate-status.md` 要。",
        "",
    ]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines), encoding="utf-8")
    try:
        shown = OUT.relative_to(REPO)
    except ValueError:      # HANDOFF_OUT 指向仓库外(检查脚本用临时目录)
        shown = OUT
    print(f"已写入 {shown}: 门禁 {len(gates)} 条 / 阶段项 {len(phases)} 项")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
