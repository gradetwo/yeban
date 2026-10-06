#!/usr/bin/env python3
"""校验 `docs/ledger/phase-status.md` 这张"每个阶段项什么状态"的表本身没有腐烂。

为什么需要一条**元判据**: 项目的目标措辞是"按 Phase -1 → Phase 0 → … → Phase 4 逐阶段交付",
而"Phase 2 还剩几项"此前没有人能一眼回答 —— 现状散在路线图、账本、状态表与 37 份工作线台账里。
这张表是那个问题的**唯一事实源**。它一旦与路线图脱节, 后果不是"少一条判据", 而是
**所有人基于错误的进度做决定**(本仓库已经踩过多次同族: L12 门禁空跑、D25 契约空转、
"人写的口径会漂移"—— 见 `docs/DEVELOPMENT_LEDGER.md` 第 12 轮)。

它与 `scripts/gates/check_gate_status.py` 是**同一套做法的第二份实现**, 但管的是不同的东西:
那张表管"发布门禁过没过"(`MUST-GATE-*`/`BASELINE-*`), 这张表管"阶段项做没做完"(`ROAD-*`)。
两张表互不复制 —— 某个 `ROAD-*` 等价于某条门禁时, 本表只写门禁 ID, 状态去门禁表读。

本脚本只做**机械可判定**的部分（语义是否正确仍要人看）:
1. 路线图 §3 里的**每一个** `ROAD-*` ID 在表里出现**恰好一次**（漏一条 = 有阶段项没人管）;
2. 每行的状态只能是 `已完成` / `部分` / `PENDING` 三者之一;
3. 每行的**证据列必须含可复跑的痕迹**（run id / `cargo ` / `bash ` / `scripts/` / `crates/` / `docs/`）——
   空证据或"我觉得做完了"不是证据;
4. **反向**也查：表里出现路线图里**没有**的 ID ⇒ 报错。凭空发明编号是硬错误
   （`AGENTS.md` §4.1 点名过先例：`MODEL-AST-006` 在规范里缺号）;
5. `PENDING` 的行必须写清**为什么**（证据列不能只有一个状态词）;
6. 末尾的**逐阶段汇总计数**必须与表格逐行统计**一致**（数字要么能被命令复核、要么别写）;
7. **标题与 §0 里的手抄总数**（`，N 项）`）、**行内的手抄 PENDING 计数**（`本表另有 **N** 项 PENDING`）
   与**标题声明的 ID 区间**也必须与逐行统计一致;
8. **其它活文档里手抄的阶段项总数**（索引 `docs/README.md` 与 `feature-alignment.md` 的分工表里
   那两处 `（47 项）` / `（`ROAD-*`，47 项）`）也必须一致 —— 第 7 条只扫本文件。
9. **本表里的行号交叉引用必须指向它点名的那一行**：`` `其它活表:NN` `` 或 `` `其它活表` 第 NN 行 ``，
   若同行**恰好点名一个**属于那张表的 ID（`HD-nn` / `MUST-GATE-nnn`·`BASELINE-nnn` / `ROAD-*`），
   则行号必须等于那个 ID 的行号。实测（本条要防的）：本表 `:43` 写 `human-decisions.md:68` 指
   `HD-34`，而 `HD-34` 在 `:70`。绑定只认**结构**（同行恰好一个 ID）；0 个或 ≥2 个一律跳过并
   登记为边界，不靠词面猜。`feature-alignment.md` 的行首是功能名、没有编号 ⇒ 引用它的行号
   （实测一处：`:105` 引 `feature-alignment.md:156`）**绑不住**，同样在本规则射程外。

**为什么第 6/7 条也在守卫里**: 汇总数字是人最爱手抄的东西, 而它恰恰是"Phase 2 还剩几项"的答案。
口径漂移在本仓库已实测发生三次以上（`docs/DEVELOPMENT_LEDGER.md` 第 12 轮）。既然能机械对账, 就不靠自觉。
第 7 条是**实测补上的洞**: 第 6 条只钉住 §7 那一份, 而"一共几项 / 还剩几项 PENDING"在这份文件里
被手抄了**五**处。表建立于 `93c83e0`（当时确实是 46 项 / PENDING 7）, 之后 `ROAD-M4-011` 入库、
PENDING 掉到 6, §7 被同步改了, 标题（×2）、`docs/README.md`、`feature-alignment.md` 与
`ROAD-M4-010` 行内那句却一直停在旧值 —— 文件自己跟自己矛盾而守卫一声不响（`ROAD-M4-010` 是本项
唯一引用的权威表, 它的数字必须能被命令复核）。

**为什么第 9 条按构造放得过"历史"**：① 它只读**本表**（citing doc）与三张**活表**（targets）；
`docs/DEVELOPMENT_LEDGER.md`、`docs/ledger/*-notes.md` 与 `docs/adr/**` **从不被打开**
⇒ 记在那里的历史够不到本规则（与 `check_decisions.py` / `check_feature_alignment.py` /
`check_gate_status.py` 同一条纪律）；② 活文档里要记录一次**移动**，必然同时给出两端
（`` 原 `…:68`，现 `…:70` ``）—— 同一行对同一目标出现**两个以上不同行号**时，那是在记录变更
而不是主张现状，按构造整行豁免（不是靠"原/曾/formerly"这类词面判断）；③ 只说"第 68 行"
而不点目标路径的散文**不是本规则认得的引用形态**，永远不是候选。

用法:
    python3 scripts/gates/check_phase_status.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TABLE = REPO / "docs/ledger/phase-status.md"
ROADMAP = REPO / "docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md"

#: 路线图里的阶段项 ID 形态：`ROAD-M-1-001`（Phase -1）与 `ROAD-M0-001` … `ROAD-M4-010`。
ROAD_ID_RE = re.compile(r"\bROAD-(?:[A-Z0-9]+-)+\d{3}\b")
#: 表格第一列里允许出现的 ID（与上面同一形态；两者不一致就是"凭空发明"）。
TABLE_ID_RE = re.compile(r"^ROAD-(?:[A-Z0-9]+-)+\d{3}$")

VALID_STATUS = ("已完成", "部分", "PENDING")

#: 证据列里必须出现这类**可复跑**的东西之一, 否则不算证据。
#: 规范点名了六种形态（run id / cargo / bash / scripts/ / crates/ / docs/），一一对应到下面。
EVIDENCE_HINTS = ("cargo ", "bash ", "scripts/", "crates/", "docs/")
RUN_ID_RE = re.compile(r"run \d{5,}")

#: 逐阶段汇总的语句形态：`- Phase 0：已完成 1 / 部分 6 / PENDING 2（共 9 项）`。
SUMMARY_RE = re.compile(
    r"^- (?:\*\*)?(Phase -1|Phase 0|Phase 1|Phase 2|Phase 3|Phase 4|合计)："
    r"已完成 (\d+) / 部分 (\d+) / PENDING (\d+)（共 (\d+) 项）"
)
#: 汇总语句 → 该阶段在 ID 上的前缀。`ROAD-M-1-` 与 `ROAD-M1-` 互不为前缀, 可以安全共存。
PHASE_PREFIX = {
    "Phase -1": "ROAD-M-1-",
    "Phase 0": "ROAD-M0-",
    "Phase 1": "ROAD-M1-",
    "Phase 2": "ROAD-M2-",
    "Phase 3": "ROAD-M3-",
    "Phase 4": "ROAD-M4-",
}

#: 文件里"手抄总数"的形态（第 7 条）：除末尾 §7 汇总的 `（共 N 项）` 之外, 另外两份副本写成
#: `，N 项）`（标题 = `（`ROAD-M-1-001` … `ROAD-M4-011`，47 项）`, §0 = `（`ROAD-*`，47 项）`）。
#: 只在 `，` 后面取数, 因此**不会**误命中 §7 的 `（共 N 项）`。
DECLARED_TOTAL_RE = re.compile(r"，\s*(\d+)\s*项）")
#: 行内手抄的 PENDING 计数（实测出现在 `ROAD-M4-010` 行的 `本表另有 **6** 项 PENDING`）。
DECLARED_PENDING_RE = re.compile(r"本表另有\s*\*\*(\d+)\*\*\s*项\s*PENDING")
#: 标题声明的 ID 区间（`（`ROAD-M-1-001` … `ROAD-M4-011`，47 项）`）—— 上界会随新阶段项漂移。
DECLARED_RANGE_RE = re.compile(r"（`(ROAD-[A-Z0-9-]+)`\s*…\s*`(ROAD-[A-Z0-9-]+)`")

#: 第 8 条：**其它活文档**里手抄的阶段项总数。第 7 条只扫本文件, 而同一个数字在
#: `docs/README.md` 的索引行与 `feature-alignment.md` 的分工表里还被各手抄了一份 ——
#: 上一轮把 `docs/README.md` 的 46 手改成 47 却没有牙, 正是这个洞。
#: ⚠ 形态必须与 `ROAD` 同处一行（下面 `[^（）\n]` 保证不跨行、不跨括号）—— 否则会把
#: `feature-alignment.md` 里 `ErrorCode::ALL，21 项）` 这类**别的**计数误判成阶段项总数。
#: 实测：活文档里 `N 项）` 共 13 处, 与 `ROAD` 同行的只有 4 处（本文件 2 + 其它 2）,
#: 且 `（共 N 项）` 形态被"`（?` 之后必须紧跟数字"天然挡住。
DECLARED_XREF_RE = re.compile(r"ROAD[^（）\n]*?[，,]?\s*（?(\d+)\s*项）")
#: 第 8 条扫的"其它活文档"（本文件自己由第 7 条负责, 不重复报）。
INDEX = REPO / "docs/README.md"
XREF_DOCS = (
    INDEX,
    REPO / "docs/ledger/feature-alignment.md",
    REPO / "docs/ledger/gate-status.md",
    REPO / "docs/ledger/human-decisions.md",
)

#: ⚠ 表格单元格里用 `\|` 转义竖线（例如把 `a | b` 的管道命令写进证据列）。
#: 朴素的 `line.split("|")` 会在**转义的**竖线上也切开, 于是整行的列都错位 ——
#: 而错位之后 `cells[2]` 拿到的不是状态, 判据会静默地判错对象。所以按"未被反斜杠转义的竖线"切。
CELL_SPLIT_RE = re.compile(r"(?<!\\)\|")

#: 第 9 条：本表里"指向另一张**活表**的行号引用"的靶子。只有**行能被机械定位**的表在射程内 ——
#: 三张表的表行首列都有一个稳定 ID：`HD-nn` / `MUST-GATE-nnn`·`BASELINE-nnn` / `ROAD-*`。
#: ⚠ `feature-alignment.md` 的行首是功能名、没有编号 ⇒ 引用它的行号绑不到某一行，按构造排除。
#: 元组 = （目标表, 表行正则, 该表的 ID 正则）。与 `check_feature_alignment.py` 第 6 条同一手法。
CITED_TABLES = (
    (
        REPO / "docs/ledger/human-decisions.md",
        re.compile(r"^\| `(HD-\d+)`"),
        re.compile(r"HD-\d+"),
    ),
    (
        REPO / "docs/ledger/gate-status.md",
        re.compile(r"^\| `((?:MUST-GATE|BASELINE)-\d+)`"),
        re.compile(r"(?:MUST-GATE|BASELINE)-\d+"),
    ),
    (
        REPO / "docs/ledger/phase-status.md",
        re.compile(r"^\| `(ROAD-[\w-]+)`"),
        re.compile(r"ROAD-[\w-]+"),
    ),
)


def cells_of(line: str) -> list[str]:
    """把一行 markdown 表格拆成单元格（正确处理 `\\|` 转义，并还原成 `|`）。"""
    body = line.strip()
    if body.startswith("|"):
        body = body[1:]
    if body.endswith("|") and not body.endswith("\\|"):
        body = body[:-1]
    return [cell.strip().replace("\\|", "|") for cell in CELL_SPLIT_RE.split(body)]


def roadmap_ids() -> list[str]:
    """从路线图里抽出全部 `ROAD-*` ID（排序后返回）。"""
    text = ROADMAP.read_text(encoding="utf-8")
    # 用 dict 去重同时保序, 再按阶段/序号排序, 让报错顺序稳定可读。
    unique = dict.fromkeys(ROAD_ID_RE.findall(text))
    return sorted(unique, key=sort_key)


def sort_key(ident: str) -> tuple[int, str]:
    """按"阶段 → 编号"排序（`ROAD-M-1-001` 在 `ROAD-M0-001` 之前）。"""
    for rank, (_, prefix) in enumerate((*PHASE_PREFIX.items(),)):
        if ident.startswith(prefix):
            return (rank, ident)
    return (len(PHASE_PREFIX), ident)


def parse_rows(text: str) -> tuple[dict[str, tuple[str, str]], list[str]]:
    """解析状态表，返回 `{ID: (状态, 证据)}` 与逐行问题清单。"""
    rows: dict[str, tuple[str, str]] = {}
    problems: list[str] = []
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = cells_of(line)
        if len(cells) < 4:
            continue
        ident = cells[0].strip("`").strip()
        if not TABLE_ID_RE.match(ident):
            continue
        if ident in rows:
            problems.append(f"{ident} 在表里出现了不止一次（一个阶段项只能有一行）")
        rows[ident] = (cells[2], cells[3])
    return rows, problems


def parse_summary(text: str) -> dict[str, tuple[int, int, int, int]]:
    """解析末尾的逐阶段汇总计数，返回 `{阶段: (已完成, 部分, PENDING, 共)}`。"""
    summary: dict[str, tuple[int, int, int, int]] = {}
    for line in text.splitlines():
        matched = SUMMARY_RE.match(line)
        if matched:
            phase = matched.group(1)
            summary[phase] = (
                int(matched.group(2)),
                int(matched.group(3)),
                int(matched.group(4)),
                int(matched.group(5)),
            )
    return summary


def table_row_lines(path: Path, row_re: re.Pattern[str]) -> dict[str, int]:
    """表里每个 ID 的**行号**（`ID -> 行号`）。表行 = 以 `|` 开头且首列就是那个 ID。"""
    return {
        matched.group(1): lineno
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
        if (matched := row_re.match(line))
    }


def cited_line_numbers(spec: str) -> set[int]:
    """把 `NN` / `NN-MM` / `NN,MM` 解析成行号集合（半角 `-` 与全角 `–` 都算区间）。"""
    numbers: set[int] = set()
    for part in spec.split(","):
        part = part.strip()
        span = re.match(r"^(\d+)\s*[-–]\s*(\d+)$", part)
        if span:
            numbers.update(range(int(span.group(1)), int(span.group(2)) + 1))
        elif part.isdigit():
            numbers.add(int(part))
    return numbers


def cross_reference_problems(text: str, citing: Path) -> tuple[list[str], int]:
    """第 9 条：本表的行号交叉引用必须指向它**点名的那一行**。

    认的形态是 `` `目标:NN` `` 与 `` `目标` 第 NN 行 ``。绑定**只认结构**：同行必须**恰好**
    出现一个属于目标表的 ID；0 个（引用散文/源码行）或 ≥2 个（绑不住是哪一个）⇒ 跳过，不猜。
    同行对同一目标给出**两个以上不同行号** ⇒ 那是在记录"移动"，按构造整行豁免（见模块文档）。
    与 `check_feature_alignment.py` 第 6 条是同一套做法的第二份实现。
    """
    problems: list[str] = []
    checked = 0
    lines = text.splitlines()
    for target, row_re, id_re in CITED_TABLES:
        rows = table_row_lines(target, row_re)
        relative = target.relative_to(REPO)
        citation_re = re.compile(
            r"`?(?:[\w./-]*/)?" + re.escape(target.name)
            + r"`?\s*(?::\s*(\d+(?:\s*[-–]\s*\d+)?(?:\s*,\s*\d+(?:\s*[-–]\s*\d+)?)*)"
            + r"|第\s*(\d+)\s*行)"
        )
        for lineno, line in enumerate(lines, 1):
            matches = list(citation_re.finditer(line))
            if not matches:
                continue
            identifiers = sorted(set(id_re.findall(line)))
            if len(identifiers) != 1:
                continue
            ident = identifiers[0]
            actual = rows.get(ident)
            if actual is None:
                continue
            stated_lines: set[int] = set()
            for matched in matches:
                stated_lines |= cited_line_numbers(matched.group(1) or matched.group(2))
            if len(stated_lines) != 1:
                continue
            stated = stated_lines.pop()
            checked += 1
            if stated != actual:
                problems.append(
                    f"{citing.relative_to(REPO)}:{lineno} 的行号引用 `{relative}:{stated}` "
                    f"指向 `{ident}`，但 `{ident}` 实际在第 {actual} 行"
                )
    return problems, checked


def main() -> int:
    if not TABLE.is_file():
        print(f"缺少阶段状态表: {TABLE}", file=sys.stderr)
        return 1
    if not ROADMAP.is_file():
        print(f"缺少权威路线图: {ROADMAP}", file=sys.stderr)
        return 1

    expected = roadmap_ids()
    if not expected:
        print(f"路线图里一个 ROAD-* ID 都没抽到: {ROADMAP}", file=sys.stderr)
        return 1

    text = TABLE.read_text(encoding="utf-8")
    rows, problems = parse_rows(text)

    # 方向 1：路线图里的每一项都必须被登记, 且状态/证据要合格。
    for ident in expected:
        if ident not in rows:
            problems.append(f"{ident} 不在表里 —— 有阶段项没人管")
            continue
        status, evidence = rows[ident]
        normalized = status.replace("*", "").strip()
        if normalized not in VALID_STATUS:
            problems.append(
                f"{ident}: 状态 `{status}` 不是 {VALID_STATUS} 之一"
                "（只许这三种词, 与 gate-status.md 同一套）"
            )
            continue
        if not evidence:
            problems.append(f"{ident}: 证据列为空 —— 没有证据的进度只是口头进度")
            continue
        if not (any(hint in evidence for hint in EVIDENCE_HINTS) or RUN_ID_RE.search(evidence)):
            problems.append(
                f"{ident}: 证据列没有可复跑的痕迹"
                "（需要 run id / `cargo ` / `bash ` / `scripts/` / `crates/` / `docs/` 之一）"
            )
        if normalized == "PENDING" and len(evidence) < 20:
            problems.append(f"{ident}: PENDING 但没写清为什么（证据列太短）")

    # 方向 2：表里不许出现路线图里没有的编号（凭空发明 = 硬错误, AGENTS.md §4.1）。
    for ident in sorted(set(rows) - set(expected), key=sort_key):
        problems.append(
            f"{ident} 在表里但**路线图里不存在** —— 凭空发明的编号是硬错误"
            "（AGENTS.md §4.1 点名过 MODEL-AST-006 的先例）"
        )

    # 方向 3：末尾的逐阶段汇总必须与逐行统计一致（数字要能被命令复核）。
    summary = parse_summary(text)
    counts_by_phase: dict[str, list[int]] = {phase: [0, 0, 0] for phase in PHASE_PREFIX}
    for ident, (status, _) in rows.items():
        for phase, prefix in PHASE_PREFIX.items():
            if ident.startswith(prefix):
                normalized = status.replace("*", "").strip()
                if normalized in VALID_STATUS:
                    counts_by_phase[phase][VALID_STATUS.index(normalized)] += 1
                break
    for phase, counts in counts_by_phase.items():
        total = sum(counts)
        if total == 0:
            continue
        if phase not in summary:
            problems.append(f"汇总里缺少 {phase} 一行（表里有 {total} 项）")
            continue
        done, partial, pending, stated_total = summary[phase]
        if [done, partial, pending] != counts or stated_total != total:
            problems.append(
                f"汇总的 {phase} 与表格不符：写的是 已完成 {done} / 部分 {partial} / "
                f"PENDING {pending}（共 {stated_total}），逐行统计是 "
                f"已完成 {counts[0]} / 部分 {counts[1]} / PENDING {counts[2]}（共 {total}）"
            )
    grand = [sum(counts[index] for counts in counts_by_phase.values()) for index in range(3)]
    stated_grand = summary.get("合计")
    if stated_grand is None:
        problems.append("汇总里缺少『合计』一行")
    elif list(stated_grand[:3]) != grand or stated_grand[3] != sum(grand):
        problems.append(
            f"汇总的合计与表格不符：写的是 已完成 {stated_grand[0]} / 部分 {stated_grand[1]} / "
            f"PENDING {stated_grand[2]}（共 {stated_grand[3]}），逐行统计是 "
            f"已完成 {grand[0]} / 部分 {grand[1]} / PENDING {grand[2]}（共 {sum(grand)}）"
        )

    # 方向 4（第 7 条）：标题 / §0 / 行内那几份**手抄副本**也必须与逐行统计一致。
    #
    # 判据取 `len(expected)`（路线图里的项数, 也是本脚本最后 `[ok]` 行打印的那个数）——
    # 让"守卫自己报的数字"与"文件里写的数字"必须相等, 这正是"能被命令复核"的定义。
    counted_total = len(expected)
    declared_totals = [
        (text[: matched.start()].count("\n") + 1, int(matched.group(1)))
        for matched in DECLARED_TOTAL_RE.finditer(text)
    ]
    if not declared_totals:
        problems.append("标题/§0 里没有可复核的总数（形如 `，N 项）`）—— 数字要么能被命令复核, 要么别写")
    for lineno, stated in declared_totals:
        if stated != counted_total:
            problems.append(
                f"第 {lineno} 行声明的总数 {stated} 与逐行统计不符（表里有 {counted_total} 项）"
            )
    for matched in DECLARED_PENDING_RE.finditer(text):
        lineno = text[: matched.start()].count("\n") + 1
        stated = int(matched.group(1))
        if stated != grand[2]:
            problems.append(
                f"第 {lineno} 行的『本表另有 {stated} 项 PENDING』与逐行统计不符"
                f"（表里有 {grand[2]} 项 PENDING）"
            )
    declared_range = DECLARED_RANGE_RE.search(text)
    if declared_range is None:
        problems.append("标题里没有可复核的 ID 区间（形如 `（`ROAD-…` … `ROAD-…`，N 项）`）")
    elif (declared_range.group(1), declared_range.group(2)) != (expected[0], expected[-1]):
        problems.append(
            f"标题声明的 ID 区间 {declared_range.group(1)} … {declared_range.group(2)} 与表里实际的 "
            f"{expected[0]} … {expected[-1]} 不符"
        )

    # 方向 5（第 8 条）：**其它活文档**里手抄的阶段项总数（索引 + 三张兄弟表）。
    for path in XREF_DOCS:
        if not path.is_file():
            problems.append(f"缺少活文档 {path.relative_to(REPO)}（第 8 条无法对账）")
            continue
        document = path.read_text(encoding="utf-8")
        for matched in DECLARED_XREF_RE.finditer(document):
            lineno = document[: matched.start()].count("\n") + 1
            stated = int(matched.group(1))
            if stated != counted_total:
                problems.append(
                    f"{path.relative_to(REPO)}:{lineno} 手抄的阶段项总数 {stated} 与逐行统计不符"
                    f"（表里有 {counted_total} 项）"
                )

    # 方向 6（第 9 条）：本表里的**行号交叉引用**必须指向它点名的那一行。
    #
    # 第 6/7/8 条管的是**手抄的数字**；本表里还有一类跨文档的活声明：`` `其它活表:NN` ``。
    # 目标表一插行，这种引用就静默变假（实测：本表 `:43` 指 `HD-34` 的引用停在 `:68`，
    # 而 `HD-34` 在 `:70`），此前没有任何守卫复核。
    # 只绑"同行恰好一个 ID"的结构；绑不住的（散文/源码行、多个 ID）按构造跳过。
    citation_problems, citations_checked = cross_reference_problems(text, TABLE)
    problems.extend(citation_problems)

    if problems:
        print("阶段状态表校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1

    done, partial, pending = grand
    print(
        f"[ok] phase-status.md: {len(expected)} 项阶段要求, "
        f"已完成 {done} / 部分 {partial} / PENDING {pending}；"
        f"{citations_checked} 处行号交叉引用全部落在目标行"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
