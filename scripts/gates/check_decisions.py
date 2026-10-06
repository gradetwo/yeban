#!/usr/bin/env python3
"""校验 `docs/ledger/human-decisions.md`：**待人类裁决的清单不能悄悄漏项或空转**。

为什么需要这条元判据: 那份清单是人类唯一的决策入口。它有两种典型的腐烂方式:
1. **漏项**: ADR 里新写了一条 `Proposed` 裁决, 却没进清单 ⇒ 人类永远看不到它, 而 Agent 照旧执行;
2. **空转**: 某行只写"请裁决", 没写**选项/建议/不决定的后果** ⇒ 人类无法判断, 只能反问,
   于是一个来回变成三个来回。

本脚本只做**机械可判定**的部分(语义仍要人看):
- ADR 里每个**标了 `Proposed`/`待人类`/`需人类` 的 `D<n>` 段落**, 必须在清单里出现;
- 清单每行必须有 6 列(ID/问题/选项/建议/后果)且 `HD-nn` 编号唯一;
- 清单必须写明"当前处置"(即: 不等裁决也能继续推进, 不构成单点阻塞);
- **手抄计数副本必须与逐行统计一致**: 清单自己的表头(`**N 项中 M 项已裁决**` / `未决 N 项`)
  与索引 / 四张表里手抄的 `HD-01..HD-NN` 区间上界, 全部对账。
  数字要么能被命令复核, 要么别写 —— 与 `check_phase_status.py` 第 7 条同一手法。
- **表头点名的"未决集合"必须等于逐行算出来的未决集合**: 不只是个数, 而是**集合本身**
  (`未决 3 项 = HD-48（…）与 HD-49（…）与 HD-50（…）`)。计数对而点错名 = 仍然在骗人:
  实测表头曾点 `HD-44`(该行早在 2026-10-05 就带 `✅ 已裁决`)而漏掉真正未决的 `HD-48` ——
  `未决 3 项` 这个**数**是对的, 所以旧守卫每次都打印 `[ok]`, 而人照表头去找 `HD-44` 会白跑一趟。
- **节标题里的"未决"也是活声明**: `## B. 契约与接口的待裁决` 说"这一节在等人类裁决",
  该节就必须至少有一条不带 `✅` 的行(实测 §B 的 11 行早已全部裁决, 标题却一直没改)。

**为什么"点名集合"也要在守卫里**: "3 项未决"只说了**有几个**, 没说**是哪几个** ——
而人真正要答的是后者。数字与点名是两份手抄副本, 对账只做了前者, 后者便悄悄腐烂:
上一片把表头从 49/47/2 改成 50/47/3 并顺手点了 `HD-50`, 却没有复核原有的 `HD-44`/`HD-49`
是否还是未决 —— 于是 `HD-44` 带着 `✅` 继续被当作"等你裁决"。(同族实测: 66/70 行、
`HD-40/42`、阶段项 46/47 都曾这样腐烂。)

**为什么"计数"也在守卫里**: 清单从 40 项长到 49 项的过程中, `HD-01..HD-40` / `HD-01..HD-42`
被手抄进 `docs/README.md`、`feature-alignment.md`、`phase-status.md`, 而没有任何判据去复核它们 ——
本脚本每次只打印自己的 `49 项`, 于是三处旧区间一直与清单矛盾而门禁全绿(实测)。
**哪些文件参与对账(这是个判据, 不是随手写的)**: 只有**活文档** —— 索引 `docs/README.md`
与四张表。`docs/DEVELOPMENT_LEDGER.md`、`docs/ledger/*-notes.md` 与 `docs/adr/**` 是
**带日期的测量/裁决记录**(ADR 那一节紧邻的一句就是 "2026-10-04 起…那 42 项"), 按纪律不得改写,
因此**不参与**对账 —— 否则守卫会逼人篡改历史读数。
"点名集合"那条**只读清单自己的表头**(它也在活文档里), 因此同样够不到任何带日期的副本:
`docs/ledger/m4-008-authority-notes.md` 记着当时的 `49 项 / 47 已裁决 / 头部自称 49 项中 47 项`,
那是**那一刻的真话**(且它按 `ledger/*-notes.md` 归类), 本规则按构造不碰它。

**点名集合怎么抽(认的是结构, 不是词面)**: 取 `未决 N 项` **之后同一逻辑行**的余文,
其中**全角括号 `（…）` 之外**的每个 `HD-nn`(带不带反引号都算)即"被点名的未决项";
`（…）` 里是**解释散文**, 可能顺带提到别的编号(现行表头在 `HD-49` 的括注里就提到 `HD-38`/`D50`),
属不可机械判定的部分, 按构造排除。这样"点错名/漏点名"必红, 而改写解释文字不会误红。
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

#: 索引：`docs/README.md` 的文档目录表。
INDEX = REPO / "docs/README.md"
#: **活文档**(数字是当下的声明): 索引 + 四张表。清单自己也在里面(表头计数)。
#: `docs/DEVELOPMENT_LEDGER.md`、`docs/ledger/*-notes.md` 与 `docs/adr/**` 是**带日期的
#: 测量/裁决记录**, 按纪律不得改写, 故不参与对账(ADR 里那处 `40 项` 紧邻 `那 42 项` 的历史叙述)。
LIVE_DOCS = (
    INDEX,
    DECISIONS,
    REPO / "docs/ledger/gate-status.md",
    REPO / "docs/ledger/phase-status.md",
    REPO / "docs/ledger/feature-alignment.md",
)

#: 手抄的 HD 区间上界: `HD-01..HD-42`。上界会随新条目漂移, 此前无人复核
#: (实测三处写 40/42, 而清单有 49 项)。
HD_RANGE_RE = re.compile(r"HD-01\.\.HD-(\d+)")
#: 清单自己的表头声明: `**49 项中 47 项已裁决**`。
DECLARED_TOTAL_DECIDED_RE = re.compile(r"\*\*(\d+)\s*项中\s*(\d+)\s*项已裁决\*\*")
#: 表头后半句: `未决 2 项`。
DECLARED_OPEN_RE = re.compile(r"未决\s*(\d+)\s*项")
#: 点名集合: `HD-nn`（带不带反引号都算; 实测表头写成 `` `HD-44` ``）。
HD_ID_RE = re.compile(r"`?(HD-\d+)`?")
#: 全角括号括注: 里面是**解释散文**, 按构造排除在"点名集合"之外
#: （现行的 `HD-49` 括注里就提到 `HD-38`/`D50`, 那是解释而非被点名的未决项）。
FULLWIDTH_PAREN_RE = re.compile(r"（[^）]*）")
#: 节标题（`## B. 契约与接口的待裁决`）。只认 `##`..`####`: 文件级 `# 标题` 是清单的名字,
#: 清单即使全部裁决完毕也仍叫这个名字, 故不参与。
SECTION_HEADING_RE = re.compile(r"^#{2,4}\s+(.+?)\s*$")
#: 节标题里出现这些词, 就是**活声明**"这一节在等人类"。它必须与该节的逐行统计一致。
OPENNESS_WORDS = ("待裁决", "未决", "待人类", "需人类")


def declared_open_ids(tail: str) -> set[str]:
    """从 `未决 N 项` **之后同一逻辑行**的余文里抽出被点名的未决编号。

    只认结构: 先挖掉所有全角括注（解释文字）, 再取剩下的 `HD-nn`。
    """
    return set(HD_ID_RE.findall(FULLWIDTH_PAREN_RE.sub("", tail)))


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
    # 判据用的标记就是每行末尾的 `✅`（维护纪律第 4 条: `✅ 已裁决 <日期> <结论>`），
    # 没有 `✅` 的行即**未决** —— 下面"点名集合"那条用的就是这个集合。
    decided = 0
    computed_open: set[str] = set()
    for line in decisions_text.splitlines():
        if not line.startswith("| `HD-"):
            continue
        ident = line.split("|")[1].strip().strip("`")
        if "✅" in line:
            decided += 1
            if not re.search(r"\d{4}-\d{2}-\d{2}", line):
                problems.append(
                    f"{line.split('|')[1].strip()}: 标了已裁决但没有日期"
                )
        else:
            computed_open.add(ident)

    # 手抄计数副本必须与逐行统计一致(数字要么能被命令复核, 要么别写)。
    # 清单自己的表头 —— 它就在这份文件里, 却没有任何判据复核过它。
    header = DECLARED_TOTAL_DECIDED_RE.search(decisions_text)
    if header is None:
        problems.append(
            "清单表头没有可复核的计数(形如 `**N 项中 M 项已裁决**`)"
            " —— 数字要么能被命令复核, 要么别写"
        )
    else:
        lineno = decisions_text[: header.start()].count("\n") + 1
        stated_total, stated_decided = int(header.group(1)), int(header.group(2))
        if (stated_total, stated_decided) != (rows, decided):
            problems.append(
                f"{DECISIONS.relative_to(REPO)}:{lineno} 表头声明的 "
                f"`{stated_total} 项中 {stated_decided} 项已裁决` 与逐行统计不符"
                f"(清单有 {rows} 项, 其中 {decided} 项已裁决)"
            )
    for matched in DECLARED_OPEN_RE.finditer(decisions_text):
        lineno = decisions_text[: matched.start()].count("\n") + 1
        stated_open = int(matched.group(1))
        if stated_open != rows - decided:
            problems.append(
                f"{DECISIONS.relative_to(REPO)}:{lineno} 声明的『未决 {stated_open} 项』"
                f"与逐行统计不符(清单有 {rows} 项, {decided} 项已裁决 ⇒ 未决 {rows - decided} 项)"
            )
        # 不只是个数, 连**点名**也要对: 计数对而点错名, 人照表头去答就会白跑一趟。
        # 取"同一逻辑行"的余文（表头把整条声明写在一行里）; 括注里的解释散文按构造排除。
        line_end = decisions_text.find("\n", matched.end())
        tail = decisions_text[matched.end() : line_end if line_end != -1 else len(decisions_text)]
        declared_open = declared_open_ids(tail)
        if declared_open != computed_open:
            wrong = sorted(declared_open - computed_open, key=lambda s: int(s[3:]))
            missing = sorted(computed_open - declared_open, key=lambda s: int(s[3:]))
            detail: list[str] = []
            if wrong:
                detail.append(
                    "声明为未决、但行内已带 ✅ 的: " + "、".join(f"`{ident}`" for ident in wrong)
                )
            if missing:
                detail.append(
                    "逐行未决、却没被点名的: " + "、".join(f"`{ident}`" for ident in missing)
                )
            problems.append(
                f"{DECISIONS.relative_to(REPO)}:{lineno} 表头点名的未决集合 "
                f"{'、'.join(f'`{ident}`' for ident in sorted(declared_open, key=lambda s: int(s[3:]))) or '(空)'} "
                f"与逐行统计不符"
                f"(逐行未决集合是 "
                f"{'、'.join(f'`{ident}`' for ident in sorted(computed_open, key=lambda s: int(s[3:]))) or '(空)'}"
                f"{'; ' + '; '.join(detail) if detail else ''})"
            )

    # 节标题同样是**活声明**: `## B. 契约与接口的待裁决` 说"这一节在等人类裁决"。
    # 一行是否未决由 `✅` 机械判定 ⇒ "标题说未决、节里却一条未决都没有"可以守卫, 不必猜。
    # 实测 §B 的 11 行(HD-20..HD-30)早在 2026-10-04 就全部带 `✅`, 标题却一直说"待裁决"。
    # ⚠ 边界: 这条把标题里的未决词**当作当下的声明**(而不是"历史分类名")。这是有意的 ——
    # 与 `check_gate_status.py` 第 6 条同一取舍: 宁可红一次让人把标题写清楚, 也不静默放过。
    current_heading: tuple[int, str] | None = None
    current_has_open = False

    def close_section() -> None:
        if current_heading is None:
            return
        heading_lineno, title = current_heading
        if any(word in title for word in OPENNESS_WORDS) and not current_has_open:
            problems.append(
                f"{DECISIONS.relative_to(REPO)}:{heading_lineno} 节标题声明的『{title}』"
                f"要求本节至少有一条未决项, 但本节 0 条未决(本节所有条目都带 ✅)"
            )

    for lineno, line in enumerate(decisions_text.splitlines(), 1):
        heading = SECTION_HEADING_RE.match(line)
        if heading:
            close_section()
            current_heading = (lineno, heading.group(1))
            current_has_open = False
            continue
        if line.startswith("| `HD-") and "✅" not in line:
            current_has_open = True
    close_section()

    # 索引与四张表里手抄的 `HD-01..HD-NN` 区间上界也要等于清单里最大的那个编号。
    max_hd = max((int(ident[3:]) for ident in seen_ids), default=0)
    if max_hd == 0:
        problems.append("清单里一个 `HD-nn` 编号都没抽到 —— 解析口径已过期")
    for path in LIVE_DOCS:
        if not path.is_file():
            problems.append(f"缺少活文档 {path.relative_to(REPO)}(HD 区间对账无法进行)")
            continue
        document = path.read_text(encoding="utf-8")
        for matched in HD_RANGE_RE.finditer(document):
            lineno = document[: matched.start()].count("\n") + 1
            stated_hd = int(matched.group(1))
            if stated_hd != max_hd:
                problems.append(
                    f"{path.relative_to(REPO)}:{lineno} 声明的 HD 区间上界 "
                    f"`HD-01..HD-{stated_hd:02d}` 与清单不符"
                    f"(清单有 {rows} 项, 上界是 `HD-01..HD-{max_hd:02d}`)"
                )

    if problems:
        print("待裁决清单校验未通过:", file=sys.stderr)
        for item in problems:
            print(f"  - {item}", file=sys.stderr)
        return 1
    summary = (
        f"[ok] human-decisions.md: {rows} 项"
        f"(其中 {decided} 项已裁决 ⇒ 未决 {len(computed_open)} 项 = "
        f"{'、'.join(f'`{ident}`' for ident in sorted(computed_open, key=lambda s: int(s[3:])))}"
        f", 与表头点名一致); "
        f"ADR 里 {len(awaiting)} 条人类标记的裁决全部在册"
    )
    print(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
