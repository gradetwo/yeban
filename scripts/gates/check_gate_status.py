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
6. **表格行里的"行内门禁状态副本"必须与本表的状态列一致** —— 活文档的表格里除了聚合数字，
   还把**单条门禁的状态**行内抄进了别的行（实测 `phase-status.md` 8 行 / 9 处、
   `feature-alignment.md` 1 处）。第 5 条只管聚合，这些**逐条**副本此前无人复核：
   实测 `MUST-GATE-014`（**PENDING**）、`BASELINE-005`（**PENDING**）、`BASELINE-004`（**部分**）、
   `MUST-GATE-001/002/011/012` 等 10 处早已与本表不符而门禁全绿。
   ⚠ 本条**只扫表格行**（首个非空白字符是 `|`）：同一串 `` `BASELINE-002`（**部分**） ``
   出现在**正文**里时是**带日期的历史记录**（`phase-status.md:51` 当场更正它、`:108` 记着
   "本行旧版本写的是…"），按纪律**不得改写** —— 改了就是篡改历史。行 = 活声明、正文 = 记录，
   这个区分是**结构性**的，正文因此**按构造**不在射程内。
   **边界（如实登记）**：本条只认**能被锚定**的行内写法（`` `ID`（…状态…） `` / `` `ID` = 状态 ``）。
   行单元格里**自由散文**式的状态词锚不住 —— 实测 `feature-alignment.md:205` 的
   「`BASELINE-005` 的『机器就绪、门禁仍 PENDING』」就属这一类（该门禁现已 `已接线`），
   本规则**不覆盖**它。不写一条靠猜的规则去够它: 一条会误判的规则比一条覆盖窄的规则危险。

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

#: 第 6 条: 行内门禁状态副本里的 ID 形态（与本表的行首列同一形态）。
INLINE_ID_PATTERN = r"(?:MUST-GATE|BASELINE)-[0-9]{3}"
#: 第 6 条认的三种**行内**状态写法（实测活文档里都出现过）:
#: ① `` `MUST-GATE-014`（**PENDING** —— 素材待人类选定） ``（括注可带原因尾巴）;
#: ② `` `MUST-GATE-012` = **部分** ``; ③ `` `MUST-GATE-002` = 部分 ``（不带粗体）。
#: ⚠ 反斜杠转义的 `\|` 由行首取样天然排除；`（**已接线**）+ 本机可复跑 …` 这种**没有 ID 前缀**
#: 的括注不会被命中（正则要求前面紧跟被反引号包住的 ID）。
INLINE_CLAIM_RE = re.compile(
    rf"`(?P<ident>{INLINE_ID_PATTERN})`\s*(?:"
    r"（(?:\*\*)?(?P<paren>已接线|部分|PENDING)(?:\*\*)?[^）]*）"
    r"|=\s*\*{0,2}(?P<equals>已接线|部分|PENDING)\*{0,2})"
)
#: 第 6 条里**唯一**的"记录豁免"形态（裁定第 2 条）: 同一表格行**自己**用"旧值 → 新值"的转写
#: 把这条 ID 的旧状态标注成历史（house style: `` 已由「部分」升为 **已接线** ``），**且新值就是**
#: 本表的当前值。两件都满足才算**记录**; 缺一、或写成别的措辞，一律按**活声明**报红 ——
#: 宁可红一次让人把话说清楚，也不静默放过一条可能与事实不符的行内声明。
#: 豁免是**逐 (行, ID, 声明值)** 的: 连"这一行把「部分」标成了历史"都要写对，才享受豁免。
#: 实测只命中 `phase-status.md:51`（该行既引用旧值 `BASELINE-002`（**部分**），又当场写下
#: 「已由「部分」升为 **已接线**」——所以它是记录而不是声明）。
INLINE_RECORD_RE = re.compile(
    rf"`?(?P<ident>{INLINE_ID_PATTERN})`?[^|]{{0,4}}"
    r"已由[「『]?(?P<old>已接线|部分|PENDING)[」』]?\s*升为\s*\*\*?(?P<new>已接线|部分|PENDING)"
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


def check_inline_status_copies(
    rows: dict[str, tuple[str, str]],
) -> tuple[list[str], int, int]:
    """第 6 条: 活文档**表格行**里行内手抄的门禁状态必须与权威表的状态列一致。

    返回 `(问题清单, 已对账的活声明数, 按裁定第 2 条豁免的历史引用数)`。

    ⚠ 只取"首个非空白字符是 `|`"的行。**正文**里同样的串（例如 `phase-status.md:51` 那句
    "两处引用已过期…"、`:108` 那句"本行旧版本写的是…"）是**记录**, 按构造不在射程内 ——
    这条规则的**安全性**正来自这个结构性区分, 而不是来自"看起来像不像声明"的词面判断。
    """
    authoritative: dict[str, str] = {}
    for ident, (status, _evidence) in rows.items():
        matched = [candidate for candidate in VALID_STATUS if candidate in status]
        if len(matched) == 1:
            authoritative[ident] = matched[0]
    problems: list[str] = []
    checked = 0
    records = 0
    for path in LIVE_DOCS:
        if not path.is_file():
            continue  # 缺文件已由第 5 条报过一次, 不在这里重复报。
        document = path.read_text(encoding="utf-8")
        for lineno, line in enumerate(document.splitlines(), 1):
            if not line.lstrip().startswith("|"):
                continue  # 正文 = 记录, 不参与。
            # 这一行把哪些 ID 的哪个旧值**当场转写**成了现在的值（逐 (ID, 旧值)）。
            recorded = {
                matched.group("ident"): matched.group("old")
                for matched in INLINE_RECORD_RE.finditer(line)
                if matched.group("new") == authoritative.get(matched.group("ident"))
            }
            for matched in INLINE_CLAIM_RE.finditer(line):
                ident = matched.group("ident")
                claimed = matched.group("paren") or matched.group("equals")
                current = authoritative.get(ident)
                if current is None:
                    problems.append(
                        f"{path.relative_to(REPO)}:{lineno} 行内副本引用了本表里没有的 "
                        f"`{ident}` —— 有门禁编号被凭空发明"
                    )
                    continue
                if claimed == current:
                    checked += 1
                    continue
                if recorded.get(ident) == claimed:
                    # 裁定第 2 条: 这一行自己把「claimed → current」写成了历史转写 ⇒ 是记录。
                    records += 1
                    continue
                problems.append(
                    f"{path.relative_to(REPO)}:{lineno} 行内副本 `{ident}` 声明「{claimed}」"
                    f"与 `gate-status.md` 不符（表里是 {current}）—— 表格行是活声明, "
                    f"要么改成 {current}, 要么在同一行写明它已是历史"
                )
    return problems, checked, records


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

    # ---- 第 6 条: 表格行里的**行内**门禁状态副本 ---------------------------------
    #
    # 第 5 条钉住的是"聚合"（区间 / 总条数 / 三状态分解），它管不到"某一行里随口抄了
    # `MUST-GATE-014`（**PENDING**）"这种**逐条**副本 —— 而那正是"文件自己跟自己矛盾"的
    # 另一种形态（实测 10 处早已腐烂而门禁全绿）。按同一手法钉住: 行内副本必须等于状态列。
    inline_problems, inline_checked, inline_records = check_inline_status_copies(rows)
    problems.extend(inline_problems)

    if problems:
        print("门禁状态表校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1
    wired, partial, pending = (status_counts[word] for word in VALID_STATUS)
    print(
        f"[ok] gate-status.md: {len(MUST_GATES)} 条 MUST-GATE + {len(BASELINES)} 条 BASELINE "
        f"均已登记且带证据/原因; 共 {total} 条 = 已接线 {wired} / 部分 {partial} / PENDING {pending}; "
        f"其它活文档的**表格行**里, 行内门禁状态副本 {inline_checked} 处与状态列一致"
        f"（另有 {inline_records} 处为当场更正过的历史引用, 按「行 = 活声明 / 正文 = 记录」的结构区分豁免）"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
