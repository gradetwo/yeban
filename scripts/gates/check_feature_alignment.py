#!/usr/bin/env python3
"""校验 `docs/ledger/feature-alignment.md`（三方对齐矩阵）本身没有腐烂。

## 为什么需要这条判据

项目的目标措辞是"系统已经实现/计划的功能，UI 端与 MCP 端都要暴露"。
在这张表之前，"某个能力在系统里有了、但界面上碰不到、也没有工具能调"这类**错位**
只散落在 42 份工作线台账的"未实现项 / 边界 / needs"小节里 —— 它**没有任何单一去处**，
于是同一件事会被三个人按三种口径判断（本仓库已实测多次：`DEVELOPMENT_LEDGER.md` 第 12 轮、
第 19 轮；`L12` 门禁空跑；`D25` 契约空转）。

本表是"三方暴露"的**唯一事实源**。它与另外三张表是**同一种做法的第四份实现**，但管的东西不同：

| 表 | 管什么 | 守卫 |
| :--- | :--- | :--- |
| 本表 `feature-alignment.md` | **三方暴露**（系统 / UI / MCP） | 本脚本 |
| `gate-status.md` | 发布门禁过没过 | `check_gate_status.py` |
| `phase-status.md` | 阶段项做没做完 | `check_phase_status.py` |
| `human-decisions.md` | 待人类裁决 | `check_decisions.py` |

## 六条机械判据（语义是否正确仍要人看）

1. **正向完整性**：`schemas/mcp-tools.schema.json` 里的**每一个** `yeban_*` 工具、
   `crates/yeban-ui-mcp/src/methods.rs` 里的**每一条** `ui/*` 方法，都必须在本表里
   **作为一行**或被某一行的 UI / MCP 列**点名**。漏一个 ⇒ 有人做了能力却没人管它暴露没有。
2. **反向硬规则**：本表里出现的**每一个**工具名 / 方法名，都必须真的在 `crates/` 里 grep 得到。
   凭空发明的名字直接红 —— 与 `scripts/gates/spec_id_audit.py` 的"不得发明 ID"同族
   （`AGENTS.md` §4.1 点名过 `MODEL-AST-006` 的先例）。
3. **结构**：每行的功能 / 系统 / UI / MCP / 缺口五列非空；三侧标记只能是允许集合内的词；
   缺口列必须含 `状态：<状态词>`；**凡"无 / 部分"的行**必须把 `原因：…；计划：…；状态：…`
   三件事按序写全、各自非空。**这是本任务的核心要求**：一个"无"若没有原因与计划，
   下一个人只会把它重问一遍。
4. **汇总对账**：顶部的六类分类计数必须与表格逐行统计**逐一相等**。
   汇总数字是人最爱手抄的东西；口径漂移在本仓库已实测发生多次。改了行忘了改汇总 ⇒ 红。
5. **手抄副本对账**：本脚本 `[ok]` 行打印的三个数字（`N 行功能` / `N 个 MCP 工具` /
   `N 条 ui 方法`）与 §4 那句 `本表有 N 行是 未核查`，在这些**活文档**里被手抄了多处。
   凡是能被命令算出来的手抄数字，一律与逐行统计对账（与 `check_phase_status.py` 第 7 条同一手法）。

**为什么第 5 条也在守卫里**：第 4 条只钉住本表 §1 那一份汇总，而同一个"一共几行"在
**索引**（`docs/README.md` 的表格行）与本表**标题**里还被手抄了两次 —— 表从 66 行长到 73 行时
两份副本都没人改，文件自己跟自己矛盾而守卫一声不响（实测：标题写 70 行、索引写 `66 行功能`，
而表里是 73 行；`HD` 区间同理，见 `check_decisions.py`）。

**哪些文件参与对账（这是个判据，不是随手写的）**：只有**活文档** —— 索引 `docs/README.md`
与四张表（本表 / `gate-status.md` / `phase-status.md` / `human-decisions.md`）。
`docs/DEVELOPMENT_LEDGER.md` 与 `docs/ledger/*-notes.md` 是**带日期的测量记录**
（"当轮测得 66 行"是那一刻的真话），按纪律不得改写，因此**不参与**对账 —— 否则守卫会逼人篡改历史读数。

6. **行号交叉引用必须指向它点名的那一行**：本表里凡是 `` `其它活表:NN` `` 或
   `` `其它活表` 第 NN 行 `` 的写法，若同行**恰好点名了一个**属于那张表的 ID
   （`HD-nn` / `MUST-GATE-nnn`·`BASELINE-nnn` / `ROAD-*`），则该行号必须等于那个 ID 的行号。
   实测（本条要防的）：`feature-alignment.md:244` 写 `human-decisions.md:36` 指 `HD-12`，
   而 `HD-12` 在 `:38`；`:86`/`:219` 写 `gate-status.md:44` 指 `BASELINE-004`，而它在 `:41`。
   这类引用是**跨文档的活声明**：目标表一插行，引用就静默变假，而它此前**没有任何守卫**。

   **为什么把范围钉死成"同行唯一 ID"**：绑定必须**结构化**，不能靠词面猜。同行 0 个 ID
   （引用的是散文/源码行）或 ≥2 个不同 ID（绑不住是哪一个）一律**跳过**并如实登记为边界 ——
   不写一条会误判的规则去够它。源码行引用（`foo.rs:NN`）同理不在射程内：把散文里的符号名
   映射到某一行需要猜，实测一条朴素的绑定器在活文档上产出 24/27 处**假红**，故不采用。
   `feature-alignment.md` 自己的行首是功能名、没有编号 ⇒ 引用它的行号同样无法机械绑定。

   **为什么 dated records 与"故意提旧位置"的散文按构造安全**：
   ① 本条只读**本表**（citing doc）与三张**活表**（targets）；`docs/DEVELOPMENT_LEDGER.md`、
   `docs/ledger/*-notes.md` 与 `docs/adr/**` **从不被打开** ⇒ 写在那里的历史够不到本规则；
   ② 活文档里要**记录一次移动**，必然同时给出两端（`` 原 `…:36`，现 `…:38` ``）—— 同一行对
   同一目标出现**两个以上不同行号**时，这一行是在记录变更而不是主张现状，按构造整行豁免；
   ③ 只说"第 36 行"而不点目标路径的散文**不是本规则认得的引用形态**，永远不是候选。

## 为什么"分类"是机器算的而不是人填的

分类由 `classify()` 从三侧标记**唯一确定**，不写在表里（表列固定为任务书要求的五列）。
这样"分类"与"三侧标记"不可能互相矛盾 —— 也就没有第二个可漂移的口径。

用法:
    python3 scripts/gates/check_feature_alignment.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TABLE = REPO / "docs/ledger/feature-alignment.md"
SCHEMA = REPO / "schemas/mcp-tools.schema.json"
METHODS = REPO / "crates/yeban-ui-mcp/src/methods.rs"
CRATES = REPO / "crates"

#: 三侧标记的允许取值（首词）。
SYSTEM_MARKS = ("已实现", "部分", "计划", "无")
SIDE_MARKS = ("有", "部分", "无")
#: 单元格内部的**全角**分隔符（因此单元格里不会出现半角竖线，markdown 表格不会被切坏）。
CELL_SEP = "｜"
#: 缺口列的允许状态词。
ALLOWED_STATUS = (
    "三方齐全",
    "PENDING",
    "未到期",
    "有意不做",
    "人类决策中",
    "待接线",
    "未核查",
)
#: 六个对齐分类（顺序 = 汇总里的顺序）。
CATEGORIES = (
    "三方齐全",
    "系统+UI（MCP 无）",
    "系统+MCP（UI 无）",
    "仅系统",
    "仅计划（系统也未实现）",
    "UI 或 MCP 独有（系统没有）",
)

#: 表里允许出现的名字形态：工具名与方法名。
TABLE_TOOL_RE = re.compile(r"yeban_[a-z_]+")
#: ⚠ 两侧的边界都是**承重**的：`crates/yeban-app/ui/dialogs/undo_tree_modal.slint` 与
#: `ui/transport.slint` 这类**文件路径**也含 `ui/<片段>`，朴素的 `ui/[a-z_]+` 会把
#: `ui/dialogs` / `ui/sidebar` / `ui/transport` 当成"方法名"，于是判据 2 会把真实存在的
#: 路径片段判成"真实名字"（假绿），而判据 1 又会把路径当成一条方法去要求点名（假红）。
#: `(?<![/\w-])` 让 `a/ui/b` 里的 `ui/b` 不参与匹配；`(?![.\w/])` 让 `ui/x.slint` 与
#: `ui/console/...` 不参与匹配。而 `` `ui/tree` `` / `"ui/tree"` / `ui/tree`（词尾）仍然匹配。
TABLE_METHOD_RE = re.compile(r"(?<![/\w-])ui/[a-z_]+(?![.\w/])")
#: 缺口列的三件套（**按序**，各自非空）。
#: ⚠ 状态词后面常常跟一句括注（`未到期（多实例是 v1.1+ 范围）`），因此字符类要把
#: 全角/半角括号也排除掉，否则 `状态` 会捕获成 `未到期（多实例是` 并被判为非法状态词。
STATUS_CHARS = r"[^\s；|（）()]+"
GAP_RE = re.compile(r"原因：(?P<reason>.+?)；计划：(?P<plan>.+?)；状态：(?P<status>" + STATUS_CHARS + r")")
STATUS_RE = re.compile(r"状态：(" + STATUS_CHARS + r")")
#: 汇总行：`- 三方齐全：15 行` 与 `- **合计：67 行**`。
TOTAL_RE = re.compile(r"^- \*\*合计：(\d+) 行\*\*$")

#: 索引：`docs/README.md` 的文档目录表。
INDEX = REPO / "docs/README.md"
#: **活文档**（数字是当下的声明）：索引 + 四张表。
#: `docs/DEVELOPMENT_LEDGER.md` 与 `docs/ledger/*-notes.md` 是**带日期的测量记录**
#: （"当轮测得 66 行"是那一刻的真话），按纪律不得改写，故不参与第 5 条对账。
LIVE_DOCS = (
    INDEX,
    TABLE,
    REPO / "docs/ledger/gate-status.md",
    REPO / "docs/ledger/phase-status.md",
    REPO / "docs/ledger/human-decisions.md",
)

#: 第 5 条：手抄副本的形态（各实测出现过一次）。
#: ⚠ 只认这五种**精确**形态，不做泛化的"任意 `N 行`"匹配 —— 表格里 `STYLE_PRESETS` 4 项 /
#: `MODES` 12 项 这类**别的**计数若被顺手当成行数，守卫会静默地判错对象。
#: 标题 `（系统 / UI / MCP，70 行）`（本表）、索引 `66 行功能`（`docs/README.md`）。
DECLARED_TITLE_ROWS_RE = re.compile(r"，\s*(\d+)\s*行）")
DECLARED_FEATURE_ROWS_RE = re.compile(r"(\d+)\s*行功能")
#: §4 的纪律句 `本表有 1 行是 未核查`（见 §16）。
DECLARED_UNCHECKED_RE = re.compile(r"本表有\s*(\d+)\s*行是")
#: 本脚本 `[ok]` 行打印的另外两个数字。当前活文档里没人手抄它们（实测 0 处），
#: 但它们是同一台机器算出来的同一族数字 —— 一旦有人手抄，就必须对得上。
DECLARED_TOOLS_RE = re.compile(r"(\d+)\s*个 MCP 工具")
DECLARED_METHODS_RE = re.compile(r"(\d+)\s*条 ui 方法")

#: 行数区间（任务书建议 40–70；给"加一行不用同时改守卫"留出空间）。
# 上限从 70 放宽到 80（负责人授权的自主决策，第 164 轮）: 工具集从 15 增到 16（D56 诊断导出），
# 新工具**必须**在矩阵里点名（否则判据自己会报"有工具却没人管它"），所以行数会持续增长。
# 这个区间的作用是抓"误删/误增整块"，不是精确值，故放宽不削弱它。
MIN_ROWS, MAX_ROWS = 40, 80

#: ⚠ 表格单元格里用 `\|` 转义竖线时不能切错（照 `check_phase_status.py` 的口径）。
CELL_SPLIT_RE = re.compile(r"(?<!\\)\|")

#: 只扫这些后缀的源文件（`crates/` 下没有别的文本形态承载工具名/方法名）。
SOURCE_SUFFIXES = (".rs", ".toml", ".slint")

#: 第 6 条：本表里"指向另一张**活表**的行号引用"的靶子。只有**行能被机械定位**的表在射程内 ——
#: 三张表的表行首列都有一个稳定 ID：`HD-nn` / `MUST-GATE-nnn`·`BASELINE-nnn` / `ROAD-*`。
#: ⚠ `feature-alignment.md` 的行首是功能名、没有编号 ⇒ 引用它的行号绑不到某一行，按构造排除。
#: 元组 = （目标表, 表行正则, 该表的 ID 正则）。
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
    """把一行 markdown 表格拆成单元格（正确处理 `\\|` 转义）。"""
    body = line.strip()
    if body.startswith("|"):
        body = body[1:]
    if body.endswith("|") and not body.endswith("\\|"):
        body = body[:-1]
    return [cell.strip().replace("\\|", "|") for cell in CELL_SPLIT_RE.split(body)]


def marker_of(cell: str, marks: tuple[str, ...]) -> str | None:
    """取单元格的三侧标记（`<标记>｜...`）；不合形状返回 `None`。"""
    for mark in marks:
        if cell == mark or cell.startswith(mark + CELL_SEP):
            return mark
    return None


class Row:
    """表格里的一行。"""

    def __init__(self, line_no: int, feature: str, system: str, ui: str, mcp: str, gap: str) -> None:
        self.line_no = line_no
        self.feature = feature
        self.system = system
        self.ui = ui
        self.mcp = mcp
        self.gap = gap

    @staticmethod
    def _exposed(mark: str) -> bool:
        """该侧标为"有 / 部分"即算暴露。"""
        return mark in ("有", "部分")

    def classify(self) -> str:
        """由三侧标记**唯一确定**分类（不依赖表里任何人写的文字）。"""
        exposed = self._exposed
        system_has = self.system in ("已实现", "部分")
        if self.system == "计划" or (self.system == "无" and not exposed(self.ui) and not exposed(self.mcp)):
            return "仅计划（系统也未实现）"
        if self.system == "无":
            return "UI 或 MCP 独有（系统没有）"
        if exposed(self.ui) and exposed(self.mcp):
            return "三方齐全"
        if exposed(self.ui):
            return "系统+UI（MCP 无）"
        if exposed(self.mcp):
            return "系统+MCP（UI 无）"
        if system_has:
            return "仅系统"
        return "仅计划（系统也未实现）"

    def has_gap(self) -> bool:
        """任一侧标为"无 / 部分"（或系统只到"计划"）⇒ 这一行必须写清三件事。"""
        return (
            self.system != "已实现"
            or self.ui != "有"
            or self.mcp != "有"
        )


def parse_rows(text: str) -> tuple[list[Row], list[str]]:
    """抽出数据行。判据：第 2 列（系统列）以某个系统标记 + 全角分隔符开头。"""
    rows: list[Row] = []
    problems: list[str] = []
    for index, line in enumerate(text.splitlines(), start=1):
        if not line.startswith("|"):
            continue
        cells = cells_of(line)
        if len(cells) < 5:
            continue
        system = marker_of(cells[1], SYSTEM_MARKS)
        if system is None or CELL_SEP not in cells[1]:
            continue
        rows.append(Row(index, cells[0], system, marker_of(cells[2], SIDE_MARKS) or "", marker_of(cells[3], SIDE_MARKS) or "", cells[4]))
    return rows, problems


def schema_tools() -> list[str]:
    """`schemas/mcp-tools.schema.json` 里 `ToolCall.name` 的 enum —— 10 个工具的唯一权威定义。"""
    document = json.loads(SCHEMA.read_text(encoding="utf-8"))
    enum = (
        document["definitions"]["ToolCall"]["properties"]["name"]["enum"]
    )
    return [name for name in enum if isinstance(name, str)]


def method_names() -> list[str]:
    """`crates/yeban-ui-mcp/src/methods.rs` 里 `MethodSpec.name` 引用的常量值 —— 14 条方法。"""
    text = METHODS.read_text(encoding="utf-8")
    constants = dict(re.findall(r'pub const (METHOD_[A-Z_]+): &str = "(ui/[a-z_]+)";', text))
    if not constants:
        raise SystemExit("methods.rs 里一条 `pub const METHOD_* = \"ui/...\"` 都没抽到 —— 解析口径已过期")
    used = re.findall(r"name: (METHOD_[A-Z_]+),", text)
    return [constants[name] for name in used if name in constants]


def crate_tokens() -> tuple[set[str], set[str]]:
    """把 `crates/` 下全部源码里出现的 `yeban_*` 与 `ui/*` 名字收集起来（判据 2 的"真实存在"）。"""
    tools: set[str] = set()
    methods: set[str] = set()
    for path in CRATES.rglob("*"):
        if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        tools.update(TABLE_TOOL_RE.findall(text))
        methods.update(TABLE_METHOD_RE.findall(text))
    return tools, methods


def parse_summary(text: str) -> tuple[dict[str, int], int | None]:
    """解析 §1 的六类计数与合计。"""
    summary: dict[str, int] = {}
    for category in CATEGORIES:
        pattern = re.compile(r"^- " + re.escape(category) + r"：(\d+) 行$")
        for line in text.splitlines():
            matched = pattern.match(line)
            if matched:
                summary[category] = int(matched.group(1))
    total = None
    for line in text.splitlines():
        matched = TOTAL_RE.match(line)
        if matched:
            total = int(matched.group(1))
    return summary, total


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
    """第 6 条：本表的行号交叉引用必须指向它**点名的那一行**。

    认的形态是 `` `目标:NN` `` 与 `` `目标` 第 NN 行 ``。绑定**只认结构**：同行必须**恰好**
    出现一个属于目标表的 ID；0 个（引用散文/源码行）或 ≥2 个（绑不住是哪一个）⇒ 跳过，不猜。
    同行对同一目标给出**两个以上不同行号** ⇒ 那是在记录"移动"，按构造整行豁免（见模块文档）。
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
    for required in (TABLE, SCHEMA, METHODS):
        if not required.exists():
            print(f"缺少权威来源: {required}", file=sys.stderr)
            return 1

    text = TABLE.read_text(encoding="utf-8")
    problems: list[str] = []

    rows, _ = parse_rows(text)
    if not rows:
        print("表里一行数据都没解析到 —— 列形状（系统列 `标记｜证据`）被破坏了", file=sys.stderr)
        return 1
    if not (MIN_ROWS <= len(rows) <= MAX_ROWS):
        problems.append(f"数据行数 {len(rows)} 落在 [{MIN_ROWS}, {MAX_ROWS}] 之外")

    # ---- 判据 3：结构 + 三件套 -------------------------------------------------
    seen: dict[str, int] = {}
    for row in rows:
        where = f"第 {row.line_no} 行（{row.feature[:28]}）"
        if not row.feature:
            problems.append(f"{where}: 功能列为空")
        if seen.setdefault(row.feature, row.line_no) != row.line_no:
            problems.append(f"{where}: 功能名与第 {seen[row.feature]} 行重复（一行一个功能）")
        for label, cell, marks in (("系统", row.system, SYSTEM_MARKS), ("UI", row.ui, SIDE_MARKS), ("MCP", row.mcp, SIDE_MARKS)):
            if not cell:
                problems.append(f"{where}: {label}列的标记不在 {marks} 里（写成 `{label}标记{CELL_SEP}载体`）")
        if not row.gap:
            problems.append(f"{where}: 缺口列为空")
            continue
        status = STATUS_RE.search(row.gap)
        if status is None:
            problems.append(f"{where}: 缺口列没有 `状态：<状态词>`")
        elif status.group(1) not in ALLOWED_STATUS:
            problems.append(f"{where}: 状态 `{status.group(1)}` 不在 {ALLOWED_STATUS} 里")
        elif status.group(1) == "三方齐全" and row.classify() != "三方齐全":
            problems.append(f"{where}: 写了 `状态：三方齐全`，但三侧标记算出来是 `{row.classify()}`")
        if row.has_gap():
            matched = GAP_RE.search(row.gap)
            if matched is None:
                problems.append(
                    f"{where}: 有 `无/部分/计划`，缺口列必须按序写全 `原因：…；计划：…；状态：…`"
                )
            else:
                if not matched.group("reason").strip("。， "):
                    problems.append(f"{where}: 有缺口但 `原因：` 是空的")
                if not matched.group("plan").strip("。， "):
                    problems.append(f"{where}: 有缺口但 `计划：` 是空的")

    # ---- 判据 1：正向完整性 ---------------------------------------------------
    table_tools = set(TABLE_TOOL_RE.findall(text))
    table_methods = set(TABLE_METHOD_RE.findall(text))

    expected_tools = schema_tools()
    expected_methods = method_names()
    for name in sorted(expected_tools):
        if name not in table_tools:
            problems.append(
                f"`{name}` 在 `schemas/mcp-tools.schema.json` 里但**表里没有点名** —— "
                "有工具却没人管它暴露没有"
            )
    for name in sorted(expected_methods):
        if name not in table_methods:
            problems.append(
                f"`{name}` 在 `crates/yeban-ui-mcp/src/methods.rs` 里但**表里没有点名** —— "
                "有方法却没人管它暴露没有"
            )

    # ---- 判据 2：反向硬规则（不许发明名字） ------------------------------------
    crate_tools, crate_methods = crate_tokens()
    for name in sorted(table_tools - crate_tools):
        problems.append(
            f"`{name}` 在表里但 **`crates/` 里 grep 不到** —— 凭空发明的工具名是硬错误"
            "（同 `spec_id_audit.py` 的『不得发明 ID』，`AGENTS.md` §4.1）"
        )
    for name in sorted(table_methods - crate_methods):
        problems.append(
            f"`{name}` 在表里但 **`crates/` 里 grep 不到** —— 凭空发明的方法名是硬错误"
        )

    # ---- 判据 4：汇总对账 -----------------------------------------------------
    derived = dict.fromkeys(CATEGORIES, 0)
    for row in rows:
        derived[row.classify()] += 1
    summary, stated_total = parse_summary(text)
    for category in CATEGORIES:
        if category not in summary:
            problems.append(f"§1 汇总缺少 `{category}` 一行（逐行统计 {derived[category]} 行）")
        elif summary[category] != derived[category]:
            problems.append(
                f"§1 汇总的 `{category}` 写的是 {summary[category]} 行，逐行统计是 {derived[category]} 行"
            )
    if stated_total is None:
        problems.append("§1 汇总缺少 `- **合计：N 行**` 一行")
    elif stated_total != len(rows):
        problems.append(f"§1 合计写的是 {stated_total} 行，实际数据行是 {len(rows)} 行")

    # ---- 判据 5：手抄副本对账 -------------------------------------------------
    #
    # 判据 4 只钉住本表 §1 那一份汇总。同一个"一共几行"在索引与本表标题里还被手抄了两次，
    # 改行数时没人会记得去改它们 —— 所以凡是能被算出来的手抄数字，一律对账。
    unchecked_rows = 0
    for row in rows:
        matched = STATUS_RE.search(row.gap)
        if matched is not None and matched.group(1) == "未核查":
            unchecked_rows += 1

    declared_forms = (
        (DECLARED_TITLE_ROWS_RE, len(rows), "行", "标题声明的行数"),
        (DECLARED_FEATURE_ROWS_RE, len(rows), "行", "手抄的功能行数"),
        (DECLARED_UNCHECKED_RE, unchecked_rows, "行", "手抄的『未核查』行数"),
        (DECLARED_TOOLS_RE, len(expected_tools), "个", "手抄的 MCP 工具数"),
        (DECLARED_METHODS_RE, len(expected_methods), "条", "手抄的 ui 方法数"),
    )
    for path in LIVE_DOCS:
        if not path.is_file():
            problems.append(f"缺少活文档 {path.relative_to(REPO)}（判据 5 无法对账）")
            continue
        document = path.read_text(encoding="utf-8")
        for pattern, computed, unit, label in declared_forms:
            for matched in pattern.finditer(document):
                lineno = document[: matched.start()].count("\n") + 1
                stated = int(matched.group(1))
                if stated != computed:
                    problems.append(
                        f"{path.relative_to(REPO)}:{lineno} 的『{label}』写的是 "
                        f"{stated}{unit}，守卫算出来是 {computed}{unit}"
                    )

    # ---- 判据 6：行号交叉引用必须指向它点名的那一行 -----------------------------
    #
    # 判据 4/5 管的是**手抄的数字**；本表里还有一类跨文档的活声明：`` `其它活表:NN` ``。
    # 目标表一插行，这种引用就静默变假（实测：指 `HD-12` 的引用停在 `:36`、指 `BASELINE-004`
    # 的引用停在 `:44`，而它们其实在 `:38` / `:41`），此前没有任何守卫复核。
    # 只绑"同行恰好一个 ID"的结构；绑不住的（散文/源码行、多个 ID）按构造跳过。
    citation_problems, citations_checked = cross_reference_problems(text, TABLE)
    problems.extend(citation_problems)

    if problems:
        print("三方对齐矩阵校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1

    print(
        f"[ok] feature-alignment.md: {len(rows)} 行功能 / "
        f"{len(expected_tools)} 个 MCP 工具 / {len(expected_methods)} 条 ui 方法全部点名，"
        + "，".join(f"{category} {derived[category]}" for category in CATEGORIES)
        + f"；{citations_checked} 处行号交叉引用全部落在目标行"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
