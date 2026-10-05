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

## 四条机械判据（语义是否正确仍要人看）

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

#: 行数区间（任务书建议 40–70；给"加一行不用同时改守卫"留出空间）。
MIN_ROWS, MAX_ROWS = 40, 70

#: ⚠ 表格单元格里用 `\|` 转义竖线时不能切错（照 `check_phase_status.py` 的口径）。
CELL_SPLIT_RE = re.compile(r"(?<!\\)\|")

#: 只扫这些后缀的源文件（`crates/` 下没有别的文本形态承载工具名/方法名）。
SOURCE_SUFFIXES = (".rs", ".toml", ".slint")


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

    if problems:
        print("三方对齐矩阵校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1

    print(
        f"[ok] feature-alignment.md: {len(rows)} 行功能 / "
        f"{len(expected_tools)} 个 MCP 工具 / {len(expected_methods)} 条 ui 方法全部点名，"
        + "，".join(f"{category} {derived[category]}" for category in CATEGORIES)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
