#!/usr/bin/env python3
"""校验 `docs/ledger/gate-status.md` 这张"什么算绿"的表本身没有腐烂。

为什么需要一条**元判据**: 那张表是人读"现在到底什么算绿"的唯一去处。
它一旦与事实脱节, 后果不是"少一条判据", 而是**所有人基于错误的现状做决定**
（本仓库已经踩过多次同族: L12 门禁空跑、D25 契约空转、`required-features` 绿着跳过）。

本脚本只做**机械可判定**的部分（语义是否正确仍要人看）:
1. `MUST-GATE-001..015` 与 `BASELINE-001..006` **各出现且仅出现一次**（漏一条 = 有门禁没人管）;
2. 每行的状态只能是 `已接线` / `部分` / `PENDING` 三者之一;
3. **非 PENDING 的行必须给出证据**（测试名 / run id / 命令 / 文件）—— 空证据是"口头绿";
4. `PENDING` 的行必须写清**为什么**还 PENDING（列里不能只有状态词）;
5. **手抄在活文档里的聚合必须与逐行统计一致** —— `MUST-GATE-001..015` / `BASELINE-001..006`
   这类 **ID 区间**、`21 条` 这类**总条数**、以及 `19 已接线 / 0 部分 / 2 PENDING` 这类
   **三状态分解**，凡是能从本表逐行算出来的手抄副本，一律对账。

**为什么第 5 条也在守卫里**: 前 4 条只保证"这 21 行自己没烂"，而"**一共几条 / 各自什么状态**"
才是本表存在的理由，它却在别处被手抄：`docs/README.md` 的两行、本文件标题、
`docs/ledger/phase-status.md` 的分工表各抄了一份 ID 区间，`phase-status.md:21` 抄了总条数，
`phase-status.md:108` 抄了三状态分解。同族的 `ROAD-*` 总数（`check_phase_status.py` 第 7/8 条）、
`HD-01..NN` 区间与 `N 行功能` / `N 个 MCP 工具` / `N 条 ui 方法`（`check_decisions.py`、
`check_feature_alignment.py`）都已按同一手法钉住，而这三份门禁聚合此前**没有任何守卫复核** ——
改一行状态忘了改副本 = 文件自己跟自己矛盾（同族实测：66/70 行、`HD-40/42`、阶段项 46/47
都曾悄悄腐烂，旧守卫对着腐烂的文件照样打印 `[ok]`）。

**哪些文件参与对账（这是个判据，不是随手写的）**：只有**活文档** —— 索引 `docs/README.md`
与四张表（本表 / `phase-status.md` / `feature-alignment.md` / `human-decisions.md`）。
`docs/DEVELOPMENT_LEDGER.md`、`docs/ledger/*-notes.md` 与 `docs/adr/**` 是**带日期的测量记录**
（"当轮读数是 21 条"是那一刻的真话），按纪律不得改写，因此**不参与**对账 ——
否则守卫会逼人篡改历史读数。
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

#: 索引: `docs/README.md` 的文档目录表。
INDEX = REPO / "docs/README.md"
#: 第 5 条扫的**活文档**（数字是当下的声明）: 索引 + 四张表。
#: `docs/DEVELOPMENT_LEDGER.md`、`docs/ledger/*-notes.md` 与 `docs/adr/**` 是**带日期的
#: 测量记录**, 按纪律不得改写, 故不参与对账 —— 否则守卫会逼人篡改历史读数。
LIVE_DOCS = (
    INDEX,
    TABLE,
    REPO / "docs/ledger/phase-status.md",
    REPO / "docs/ledger/feature-alignment.md",
    REPO / "docs/ledger/human-decisions.md",
)

#: 第 5 条: 手抄的 **ID 区间**（`MUST-GATE-001..015` / `BASELINE-001..006`; 实测活文档里 4 处）。
DECLARED_RANGE_RE = re.compile(r"(MUST-GATE|BASELINE)-(\d{3})\.\.(\d{3})")
#: 第 5 条: 与门禁 ID **同行**、由逗号引出的**总条数**
#: （`` `MUST-GATE-001..015` / `BASELINE-001..006`，21 条） ``; 实测活文档里 1 处）。
#: ⚠ 只认这一种**精确**形态（全角 `，` + 全角 `）`）: 活文档里还有大量与门禁无关的 `N 条`
#: 计数（例: `check_docs_links.py` 的 6 条 `[warn]`、`flate2` 的依赖条数），泛化成
#: "任意 `N 条`" 会静默地判错对象（`check_feature_alignment.py` 第 5 条有同款注释）。
DECLARED_TOTAL_RE = re.compile(r"(?:MUST-GATE|BASELINE)[^（）\n]*?[，,]\s*(\d+)\s*条）")
#: 第 5 条: 手抄的**三状态分解**（`19 已接线 / 0 部分 / 2 PENDING`; 实测活文档里 1 处）。
#: ⚠ 本表表头的纪律句 `` `已接线` / `部分` / `PENDING` `` 前面没有数字, 天然不被命中。
DECLARED_BREAKDOWN_RE = re.compile(
    r"(\d+)\s*已接线\s*/\s*(\d+)\s*部分\s*/\s*(\d+)\s*PENDING"
)


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

    # ---- 第 5 条: 手抄聚合对账 ------------------------------------------------
    #
    # 上面 4 条只保证这 21 行自己没烂; 而"一共几条 / 各自什么状态"才是本表存在的理由,
    # 它却被手抄进了别的活文档(索引两行 + 本文件标题 + `phase-status.md` 分工表各一份 ID 区间,
    # `phase-status.md` 里还各有一份总条数与三状态分解)。凡是能从 `rows` 逐行算出来的
    # 手抄数字, 一律对账 —— 数字要么能被命令复核, 要么别写。
    total = len(rows)
    status_counts = dict.fromkeys(VALID_STATUS, 0)
    for status, _evidence in rows.values():
        matched = [candidate for candidate in VALID_STATUS if candidate in status]
        if len(matched) == 1:
            status_counts[matched[0]] += 1
    #: 每一族的实际区间（`MUST-GATE-001..015`）由表里该族的**最小 / 最大**编号算出,
    #: 不写死在这里 —— 否则加一条门禁只需改守卫, 副本照样漂移。
    numbers_by_family: dict[str, list[int]] = {}
    for ident in rows:
        family, _, number = ident.rpartition("-")
        numbers_by_family.setdefault(family, []).append(int(number))
    range_edges = {
        family: (min(numbers), max(numbers)) for family, numbers in numbers_by_family.items()
    }

    for path in LIVE_DOCS:
        if not path.is_file():
            problems.append(f"缺少活文档 {path.relative_to(REPO)}(第 5 条聚合对账无法进行)")
            continue
        document = path.read_text(encoding="utf-8")
        for matched in DECLARED_RANGE_RE.finditer(document):
            lineno = document[: matched.start()].count("\n") + 1
            family = matched.group(1)
            low, high = int(matched.group(2)), int(matched.group(3))
            edge = range_edges.get(family)
            if edge is None or (low, high) != edge:
                actual = "表里没有这一族" if edge is None else f"{family}-{edge[0]:03d}..{edge[1]:03d}"
                problems.append(
                    f"{path.relative_to(REPO)}:{lineno} 手抄的 ID 区间 "
                    f"`{family}-{low:03d}..{high:03d}` 与逐行统计不符(本表里实际是 {actual})"
                )
        for matched in DECLARED_TOTAL_RE.finditer(document):
            lineno = document[: matched.start()].count("\n") + 1
            stated = int(matched.group(1))
            if stated != total:
                problems.append(
                    f"{path.relative_to(REPO)}:{lineno} 手抄的门禁总条数 {stated} 与逐行统计不符"
                    f"(本表里有 {total} 条)"
                )
        for matched in DECLARED_BREAKDOWN_RE.finditer(document):
            lineno = document[: matched.start()].count("\n") + 1
            stated = tuple(int(matched.group(index)) for index in (1, 2, 3))
            computed = tuple(status_counts[word] for word in VALID_STATUS)
            if stated != computed:
                problems.append(
                    f"{path.relative_to(REPO)}:{lineno} 手抄的三状态分解 "
                    f"{stated[0]} 已接线 / {stated[1]} 部分 / {stated[2]} PENDING 与逐行统计不符"
                    f"(本表里是 {computed[0]} 已接线 / {computed[1]} 部分 / {computed[2]} PENDING)"
                )

    if problems:
        print("门禁状态表校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1
    wired, partial, pending = (status_counts[word] for word in VALID_STATUS)
    print(
        f"[ok] gate-status.md: {len(MUST_GATES)} 条 MUST-GATE + {len(BASELINES)} 条 BASELINE "
        f"均已登记且带证据/原因; 共 {total} 条 = 已接线 {wired} / 部分 {partial} / PENDING {pending}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
