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

（上表的"守卫"列指**该表自身内容**的守卫。本脚本的判据 5 早已读**全部**活文档；
判据 7 自 2026-10-07 起也读本表 **+ `gate-status.md` + `phase-status.md`** 的源码行引用 ——
为什么扩在这里而不是那两张表各自的守卫里，见 `SOURCE_CITATION_DOCS` 的注释。）

## 七条机械判据（语义是否正确仍要人看）

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
   **同一条能力不许有两个口径**：功能名逐字相同（`seen`）或**归一化后互为包含**
   （`capability_key()`，被包含的名字 ≥ `CAPABILITY_KEY_MIN` 字符，见该函数的注释）的两行，
   三侧标记必须一致。只认"逐字相同"是不够的 —— 实测：同一条撤销 / 重做被写成
   `**执行**撤销 / 重做（可逆能力对外可调用）`（旧读数：UI `部分` / MCP `无` / `PENDING`）与
   `撤销 / 重做（yeban_undo / yeban_redo + UI 入口）`（三侧齐全）两行，守卫当时全绿。
   本判据**只在标记不一致时开火**，且实测活表上只有那 1 对候选、0 处假红
   （宽松到"最长公共子串 ≥ 5"会产出 30 对假红，故不采用模糊匹配）。
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
   不写一条会误判的规则去够它。**源码行引用（`foo.rs:NN`）由判据 7 单独管**：把散文里的
   符号名映射到某一行确实要猜，实测一条朴素的绑定器在活文档上产出 24/27 处**假红**，
   故判据 7 只认作者**主动**写出的紧邻代码括注。
   `feature-alignment.md` 自己的行首是功能名、没有编号 ⇒ 引用它的行号同样无法机械绑定。

   **为什么 dated records 与"故意提旧位置"的散文按构造安全**：
   ① 本条只读**本表**（citing doc）与三张**活表**（targets）；`docs/DEVELOPMENT_LEDGER.md`、
   `docs/ledger/*-notes.md` 与 `docs/adr/**` **从不被打开** ⇒ 写在那里的历史够不到本规则；
   ② 活文档里要**记录一次移动**，必然同时给出两端（`` 原 `…:36`，现 `…:38` ``）—— 同一行对
   同一目标出现**两个以上不同行号**时，这一行是在记录变更而不是主张现状，按构造整行豁免；
   ③ 只说"第 36 行"而不点目标路径的散文**不是本规则认得的引用形态**，永远不是候选。

7. **源码引用必须点名一个真的存在的构件**（**2026-10-08 改口径**：由"必须落在第 `NN` 行"
   改成"**按符号**"；改动的原规则、阻塞、授权与红线状态记录在
   `docs/DEVELOPMENT_LEDGER.md` 的当轮条目里）：本表里每一处 `` `源码.rs:NN` `` /
   `` `界面.slint:NN` ``，若**紧邻**它有一个**以代码片段开头**的括注
   （`` …`ids.rs:26`（`PPQ=960`）… ``），则该括注里的**第一个**反引号片段就是它点名的
   构件；守卫检查**该构件在目标文件里存在**（任意一行），**不再**要求它落在 `NN` 那一行：

   - **符号是判据**：构件被删除或被改名 ⇒ 红（"引用的对象没了"是真腐烂）；
   - **行号是提示**：符号对、行号漂了（含越过文件末尾）⇒ **不红**，只进 `[ok]` 行的
     "行号已漂移"计数。

   **为什么改**（旧口径实测的代价）：行号是**位置**，不是**身份**。任何在引用点之前插入
   代码的切片都会让一批行号静默变假，而修法（按新行号手改文档）与代码改动一一绑定，
   下一轮还会漂 —— 实测：本次在 `crates/yeban-app/src/host.rs` 里加一条能力之后，
   3 个锚点（`wire_input` / `Action::Undo` / `on_undo_step`）整体下移，5 条引用立刻变红；
   而按旧口径"修好"它们只能靠删既有注释来凑零行（净增量最小 +4 行）。改成按符号之后，
   **抓引用腐烂的能力保留**（删 / 改名仍然红），"代码长了就红"这条噪音消失。
   旧口径落规则那一刻的读数（12 处错、0 处假红）是历史真话，留在账本里不改。

   **为什么只认"紧邻的、以代码开头的括注"**：构件名与行号的绑定必须是**结构**的。
   本仓库试过"把同行散文里的符号名映射到行号"，在活文档上产出 24/27 处假红（见判据 6）；
   而紧邻括注里的第一个代码片段是作者**主动**给出的命名。拿它去目标文件里核对，落这条规则时
   实测：120 处引用里核对 33 处、其中 12 处为真错、**0 处假红**；其余 87 处按下列边界跳过。
   这 4 个数是**落规则那一刻**的读数，不是常数 —— `[ok]` 行每次打印**当前**的
   "已核对 / 跳过"两数（文档一改就会动）。

   **本条诚实的边界（判断不了的一律跳过并计数，不假装通过）**：
   ① 路径不能在工作树里**按仓库根**唯一定位的 —— 裸文件名（`commit.rs:342`）、crate 相对路径
      （`src/lib.rs:73`）、上游路径（`i-slint-core-*/item_tree.rs`）⇒ 跳过。判据**不做**
      "按 crate 猜"的解析：裸文件名与 crate 相对路径一律不绑，**哪怕该文件名在树里只出现一次**
      （实测：`phase-status.md` / `gate-status.md` 的 `host.rs:130-140` 就是这一类，工作树里只有
      `crates/yeban-app/src/host.rs` 一个候选，但本规则仍不认它 ⇒ **它挡不住这类引用的回归**，
      本切片只能手工核对后修正）。但 `` `crates/` `` / `` `docs/` `` / `` `schemas/` `` /
      `` `scripts/` `` / `` `spikes/` `` / `` `assets/` `` 开头的路径**必须**存在，
      不存在即红（文件被删/改名是真错误）；
   ② 引用旁边没有"以代码片段开头/结尾的括注"的 —— 纯 `foo.rs:NN`，或括注是散文
      （`` （4 个场景，投影自演示工程的 `scenes`） ``）⇒ 跳过。散文括注里的词是**描述**，
      不是命名；把它当构件名会误判（实测：`scene.rs:38,56` 的括注里 `scenes` 一词
      属于说明句，指错对象的是提取器而不是文档）；
   ③ 括注的第一个代码片段抽不出标识符（纯数字 / 标点）⇒ 跳过；
   ④ **射程 = 本表 + `gate-status.md` + `phase-status.md`**（本切片由"只读本表"扩到这两张
      兄弟活表，见 `SOURCE_CITATION_DOCS`；为什么扩在**本脚本**而不是那两张表各自的守卫里，
      见该常量的注释）。扩射程前实测：这两张表里的源码行引用**同样在烂** ——
      `phase-status.md` 的 `input.rs:219-225` 点名 `View::Session` / `View::Arrangement`
      （实际在 `528-532`）、`piano_roll.slint:68-70` 点名三个 `accessible-*`（实际在 `77-79`）、
      `host.rs:130-140` 点名"只注入可见窗口"（实际在 `157-184`）、`gate-status.md` 的
      `host.rs:130-140` 同理。其中 `input.rs` 那处正是②认的"紧邻括注"形态 ⇒ **规则一上就能抓**；
      `host.rs:130-140` 属边界①（裸文件名）⇒ **本规则够不到**，只能手工核对。够不到的一律
      计数、不假装通过：`[ok]` 行按表打印"核对 / 跳过"两个数。

   **为什么没有顺手把绑定器也放宽**（本切片实测，避免下一个人重做这个实验）：
   ① "同子句里引用前后第一个代码片段"这种松绑定 ⇒ 12 个候选里**手核出 7 个假红**
      （取到的是相邻散文里的别的片段，例如 `scene.rs:121`、`render.rs:6,10`），与判据 6
      记下的 24/27 假红同族；② 真正紧的"紧随其后、中间只有空白"一档，在三张活表上只多出
      **5** 处绑定、0 处假红 —— 样本太薄，不足以改判据定义。故本切片只放宽**射程**（同一手法）。

   **它能证明什么、不能证明什么**：它只能证明"作者点名的那个构件的标识符确实出现在被引行"，
   是**下界**。同一处引用点名多个构件时，只要有一个在，就算过 —— 它抓的是"整段漂走 / 指到
   无关行"这类腐烂，不保证每一行的语义都对。

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

#: 判据 3 的"同一能力"归一化名的**最小长度**。比它短的名字（实测活表里只有 `走带` 一个，
#: 归一化后 2 字符）太容易"被别的名字包含" ⇒ 按构造跳过，不拿它去制造假红。
CAPABILITY_KEY_MIN = 4

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

#: 判据 7：本表里的源码行号引用（只认这两种承载代码的后缀）。
SOURCE_CITATION_RE = re.compile(
    r"(?P<path>[A-Za-z0-9_][A-Za-z0-9_./-]*\.(?:rs|slint))"
    r":(?P<lines>\d+(?:\s*[-–]\s*\d+)?(?:\s*,\s*\d+(?:\s*[-–]\s*\d+)?)*)"
)
#: 括注里的反引号代码片段（`PPQ=960` / `View::Session` / `track-{i}-…-lane`）。
CODE_SPAN_RE = re.compile(r"`([^`]+)`")
#: 代码片段里的标识符（`PPQ=960` ⇒ `PPQ`；`CommitGraph::undo` ⇒ 两个）。
SYMBOL_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
#: 规范 ID 形态（`UI-MCP-002`）不是代码符号，不参与判据 7 的行内匹配。
SPEC_ID_RE = re.compile(r"[A-Z]+-[A-Z0-9-]+")
#: 数字字面量的下划线后缀（`10_000` ⇒ `_000`）不是符号名，同上。
NUMERIC_TAIL_RE = re.compile(r"_?\d[\d_]*")
#: 判据 7 的边界 ④：只有这些**仓库根**前缀下的路径必须存在，不存在即红。
REPO_ROOT_PREFIXES = ("crates/", "docs/", "schemas/", "scripts/", "spikes/", "assets/")

#: 判据 7 的**射程**：本表 + 两张同族活表（2026-10-07 由"只读本表"扩到这里）。
#:
#: **为什么扩在 `check_feature_alignment.py`，而不是 `check_phase_status.py` /
#: `check_gate_status.py`**：判据 7 的**实现**（`SOURCE_CITATION_RE` 提取器 +
#: `named_construct` 绑定器 + 跳过分类 + 计数）**就是这条规则本身**；把射程从本表扩到两张
#: 兄弟表是**数据**变化，不是新规则。落到那两张表各自的守卫里，都要把这套机器**再抄一遍**
#: —— 而本仓库的纪律是"同一事实出现两处，必然有一处是错的"（`check_gate_status.py` 模块
#: 文档原话）。那两张表**各自只有一个表主**，所以无论选哪一张，它都要去读**另一张**表：
#: "脚本名与表的对应"这个看似最强的理由，在需要覆盖**两张**表时对其中一张必然失效。
#: 而本脚本早已为判据 5 读**全部**活文档（`LIVE_DOCS`），跨表读文档在这里不是新事；
#: 判据 7 的"已核对 / 跳过"报数格式也因此保持一致。
#:
#: **没把绑定器一起放宽**的理由（本切片实测）见模块文档判据 7 的边界 ④ 之后那段。
SOURCE_CITATION_DOCS = (
    TABLE,
    REPO / "docs/ledger/gate-status.md",
    REPO / "docs/ledger/phase-status.md",
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


def capability_key(feature: str) -> str:
    """功能名的**归一化**形态：去掉括注与一切非词符（markdown 星号 / 反引号 / 空白 / 标点）。

    这是判据 3 里"一行一个功能"的推广。逐字相同的检查**只认字面**，于是同一条能力被
    两种写法各写一行时守卫一声不响 —— 实测（本规则落地的这一轮）：本表有
    `**执行**撤销 / 重做（可逆能力对外可调用）`（第 86 行，当时记 UI `部分` / MCP `无` / `PENDING`）
    与 `撤销 / 重做（yeban_undo / yeban_redo + UI 入口）`（第 107 行，记三侧齐全）两行，
    指的是**同一条能力**（`ADR-0001 D45` 的同一次落地），三侧标记却相反。
    归一化后 `撤销重做` 被 `执行撤销重做` 包含 ⇒ 认作同一能力，标记必须一致。

    **为什么不做模糊匹配**：本切片实测过"最长公共子串"这一档 —— 阈值放到 5 个字符就
    产出 **30 对**候选（`yeban_*` 工具名共享前缀、`.yeban` 族共享字面），阈值放到 6 仍有 9 对，
    全是假红。故本规则只认**包含**（比公共子串严格得多），并要求被包含的那个名字
    ≥ `CAPABILITY_KEY_MIN` 个字符。
    """
    without_notes = re.sub(r"（[^）]*）|\([^)]*\)", "", feature)
    return re.sub(r"[^0-9A-Za-z\u4e00-\u9fff]+", "", without_notes)


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


def _parenthetical_after(text: str, end: int) -> str | None:
    """引用结束处之后**紧邻**的括注内容（跳过引用自身的收尾反引号与空白）。"""
    cursor = end
    if cursor < len(text) and text[cursor] == "`":
        cursor += 1
    while cursor < len(text) and text[cursor] in " \t":
        cursor += 1
    if cursor >= len(text) or text[cursor] not in "（(":
        return None
    opener = text[cursor]
    closer = "）" if opener == "（" else ")"
    depth = 0
    for scan in range(cursor, len(text)):
        if text[scan] == opener:
            depth += 1
        elif text[scan] == closer:
            depth -= 1
            if depth == 0:
                return text[cursor + 1 : scan]
    return None


def _parenthetical_before(text: str, start: int) -> str | None:
    """引用开始处之前**紧邻**的括注内容（跳过引用自身的起始反引号与空白）。"""
    cursor = start
    if cursor > 0 and text[cursor - 1] == "`":
        cursor -= 1
    while cursor > 0 and text[cursor - 1] in " \t":
        cursor -= 1
    if cursor == 0 or text[cursor - 1] not in "）)":
        return None
    closer = text[cursor - 1]
    opener = "（" if closer == "）" else "("
    depth = 0
    for scan in range(cursor - 1, -1, -1):
        if text[scan] == closer:
            depth += 1
        elif text[scan] == opener:
            depth -= 1
            if depth == 0:
                return text[scan + 1 : cursor - 1]
    return None


def named_construct(text: str, start: int, end: int) -> str | None:
    """引用 `[start, end)` 紧邻括注里点名的构件（第一个代码片段）；认不出返回 `None`。

    只认**以代码片段开头**（后置括注）或**以代码片段结尾**（前置括注）的括注：
    散文括注（`` （4 个场景，投影自演示工程的 `scenes`） ``）不构成命名，按构造跳过 ——
    那是**描述**而不是构件名，拿它去核对被引行只会误判。
    """
    after = _parenthetical_after(text, end)
    if after is not None and after.lstrip().startswith("`"):
        spans = CODE_SPAN_RE.findall(after)
        return spans[0] if spans else None
    before = _parenthetical_before(text, start)
    if before is not None and before.rstrip().endswith("`"):
        spans = CODE_SPAN_RE.findall(before)
        return spans[-1] if spans else None
    return None


def source_line_problems(path: Path) -> tuple[list[str], int, int, int]:
    """判据 7：射程内**每一张**活表的源码引用必须点名一个**真的存在**的构件。

    返回 `(问题列表, 已核对处数, 按构造跳过处数, 行号已漂移处数)`。跳过的一律**不假装
    通过**：路径不能唯一定位、旁边没有代码括注、括注抽不出标识符 —— 三类都计入跳过并在
    `[ok]` 行报数。

    **符号是判据、行号是提示**（2026-10-08 改口径，理由见模块头第 7 条）：构件在目标文件里
    **任意一行**存在即通过；行号不参与成败判定，只统计漂移（漂了**不红**）。
    """
    problems: list[str] = []
    checked = 0
    skipped = 0
    hints = 0
    text = path.read_text(encoding="utf-8")
    citing = path.relative_to(REPO)
    for matched in SOURCE_CITATION_RE.finditer(text):
        lineno = text[: matched.start()].count("\n") + 1
        path = matched.group("path")
        spec = matched.group("lines")
        target = REPO / path
        if not target.is_file():
            # 只有仓库根前缀下的路径必须存在；裸文件名 / crate 相对路径（`src/lib.rs`）
            # 在工作树里有多个同名文件，绑到哪一个都是猜 ⇒ 跳过。
            if path.startswith(REPO_ROOT_PREFIXES):
                checked += 1
                problems.append(
                    f"{citing}:{lineno} 的源码行引用 `{path}:{spec}` 找不到文件"
                    f"（`{path}` 不在工作树里）"
                )
            else:
                skipped += 1
            continue
        construct = named_construct(text, matched.start(), matched.end())
        if construct is None:
            skipped += 1
            continue
        symbols = [
            symbol
            for symbol in SYMBOL_RE.findall(construct)
            if not SPEC_ID_RE.fullmatch(symbol) and not NUMERIC_TAIL_RE.fullmatch(symbol)
        ]
        if not symbols:
            skipped += 1
            continue
        checked += 1
        source = target.read_text(encoding="utf-8", errors="ignore").splitlines()
        cited = cited_line_numbers(spec)
        named = " / ".join(f"`{symbol}`" for symbol in symbols)

        def mentioned(symbol: str, line: str) -> bool:
            """该行里是否出现这个标识符（按**词边界**，免得 `undo` 命中 `undoable`）。"""
            return re.search(r"\b" + re.escape(symbol) + r"\b", line) is not None

        # **符号是判据**：构件必须在文件里存在（哪一行都行）。
        present = {
            index
            for index, line in enumerate(source, 1)
            if any(mentioned(symbol, line) for symbol in symbols)
        }
        if not present:
            problems.append(
                f"{citing}:{lineno} 的源码引用 `{path}:{spec}` 点名 {named}，"
                f"但 `{path}` 里没有它（构件被删除或被改名）"
            )
            continue
        # **行号是提示**：符号对而行号漂了（含越界）⇒ 只计数，不红。
        if not cited or not (cited & present):
            hints += 1
    return problems, checked, skipped, hints


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

    # ---- 判据 3（续）：同一条能力的两行不许给出互相矛盾的标记 -------------------
    #
    # "一行一个功能"此前只由逐字相同的功能名把守。实测它挡不住这一种腐烂：同一条能力
    # 用两种写法各写一行（第 86 行 vs 第 107 行的撤销 / 重做），一行已按落地改完、
    # 另一行停在旧读数，于是本表自己跟自己矛盾而守卫全绿。
    # 只认**包含**这一种结构关系 + 只在标记**不一致**时开火 ⇒ 实测活表上只有 1 对候选
    # （就是那对撤销 / 重做），0 处假红；修好之后两行标记一致，本判据保持沉默。
    ability_rows = [
        (row.line_no, capability_key(row.feature), row.system, row.ui, row.mcp)
        for row in rows
        if len(capability_key(row.feature)) >= CAPABILITY_KEY_MIN
    ]
    for index, (line_a, key_a, system_a, ui_a, mcp_a) in enumerate(ability_rows):
        for line_b, key_b, system_b, ui_b, mcp_b in ability_rows[index + 1 :]:
            if key_a == key_b:
                shorter, longer = key_a, key_b
            elif key_a in key_b:
                shorter, longer = key_a, key_b
            elif key_b in key_a:
                shorter, longer = key_b, key_a
            else:
                continue
            if (system_a, ui_a, mcp_a) == (system_b, ui_b, mcp_b):
                continue
            problems.append(
                f"第 {line_a} 行与第 {line_b} 行像是**同一条能力**（归一化功能名 `{shorter}` "
                f"⊆ `{longer}`），三侧标记却相反："
                f"({system_a} / {ui_a} / {mcp_a}) vs ({system_b} / {ui_b} / {mcp_b}) —— "
                "同一条能力的两行必须给出同一套标记；若确是两件事，请把功能名改到不互相包含"
            )

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

    # ---- 判据 7：射程内活表的源码行号引用必须指向它点名的构件 -------------------
    #
    # 判据 6 只绑"本表 → 其它活表"的 ID 行号；`` `inspect.rs:71` `` 这类**源码文件**的
    # 行号此前一条守卫都没有 —— 上次切片手工修掉三处漂移，只因判据 6 的射程够不到源码。
    # 只认"紧邻的、以代码片段开头/结尾的括注"这一种结构化命名；绑不住的按构造跳过并计数。
    # 本切片把射程由"只读本表"扩到 `SOURCE_CITATION_DOCS`（本表 + `gate-status.md` +
    # `phase-status.md`）—— 后两张表实测也在烂（`input.rs:219-225` 应为 `528-532`、
    # `piano_roll.slint:68-70` 应为 `77-79`、`host.rs:130-140` 应为 `157-184`），
    # 而此前没有任何守卫读它们的源码行引用。
    source_problems: list[str] = []
    source_per_doc: list[tuple[Path, int, int]] = []
    for citing_doc in SOURCE_CITATION_DOCS:
        if not citing_doc.is_file():
            problems.append(f"缺少活表 {citing_doc.relative_to(REPO)}（判据 7 无法复核其源码行引用）")
            continue
        doc_problems, doc_checked, doc_skipped, doc_hints = source_line_problems(citing_doc)
        source_problems.extend(doc_problems)
        source_per_doc.append((citing_doc, doc_checked, doc_skipped, doc_hints))
    problems.extend(source_problems)

    if problems:
        print("三方对齐矩阵校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1

    source_checked = sum(checked for _, checked, _, _ in source_per_doc)
    source_skipped = sum(skipped for _, _, skipped, _ in source_per_doc)
    source_hints = sum(hints for _, _, _, hints in source_per_doc)
    source_breakdown = " / ".join(
        f"{doc.name} 核对 {checked} 跳过 {skipped} 行号漂移 {hints}"
        for doc, checked, skipped, hints in source_per_doc
    )
    print(
        f"[ok] feature-alignment.md: {len(rows)} 行功能 / "
        f"{len(expected_tools)} 个 MCP 工具 / {len(expected_methods)} 条 ui 方法全部点名，"
        + "，".join(f"{category} {derived[category]}" for category in CATEGORIES)
        + f"；{citations_checked} 处行号交叉引用全部落在目标行"
        + f"；{source_checked} 处源码引用点名了**存在**的构件"
        + f"（另有 {source_skipped} 处路径歧义 / 未点名构件，按构造跳过、未假装通过；"
        + f"{source_hints} 处行号已漂移 —— 按新口径只提示、不判红；"
        + f"分表：{source_breakdown}）"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
