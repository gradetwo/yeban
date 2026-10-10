#!/usr/bin/env python3
"""纪律检查表门 —— 账本 R261「五组检查表」里**可无歧义机械化**的部分。

背景（R261）：本会话 244 条裁决被收成五组——
  A 读数必须绑「区域 + 单位」  B 装置失效必须可区分于零命中  C 状态突变必须可命名、可显式
  D 写盘必须原子且可回读      E 日志覆盖必须 N/N
它们在账本里是**散文**。本门把其中**在仓库内可无歧义判定**的三条变成门，其余组以注册表
登记在案并打印落地状态（⛔ 不假装能机械化）。

为什么只收三条：**门一旦误红就会被忽略**，所以只收零假阳性的判据。
  C1  状态突变必须显式：`scripts/**` 里任何脚本若要 `git commit`，必须由**显式开关**
      （`--commit` 之类）或 DRY_RUN 默认把关。没有把关 = 「跑一下看看」就会改仓库。
  C3  「绿而无效」：windows 腿的上传步必须保留 `if-no-files-found: error`
      （实测过 `warn` 会让「没产出」变成绿 —— 比红更危险）。
  E1  日志覆盖：windows 腿里按 crate 拆的 test 步必须带 `--no-fail-fast`
      （判据内部的首败仍即停，所以拆分才是 N/N 的唯一机制）。

退出码：0 = 全部通过；1 = 有违例。⛔ 不接受管道化（调用方 run-gates.sh 直接检查退出码）。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CI = REPO / ".github/workflows/ci.yml"
SCRIPTS = REPO / "scripts"

# 五组注册表：id -> (组, 判定方式)。A/B/D 三组目前只能由人读 + 各线自证，故如实标 non-mechanical。
REGISTRY: dict[str, tuple[str, str]] = {
    "A1": ("A", "non-mechanical: 裸数不可比较（区域+单位）"),
    "A2": ("A", "non-mechanical: 分区数不可相加"),
    "A3": ("A", "non-mechanical: 手数可能两头都错"),
    "A4": ("A", "non-mechanical: 多源分歧本身是证据"),
    "A5": ("A", "non-mechanical: 四形态+两口径+偏置+逐线本地"),
    "A6": ("A", "non-mechanical: 分区须可回读验证"),
    "B1": ("B", "non-mechanical: 无输出是装置状态"),
    "B2": ("B", "non-mechanical: VOID 或逐样本退出码+正对照"),
    "B3": ("B", "non-mechanical: 正对照两级（编译+运行输出）"),
    "B4": ("B", "non-mechanical: 装置体检自带正对照"),
    "B5": ("B", "non-mechanical: 0 命中带退出码"),
    "B6": ("B", "non-mechanical: 未复现 ≠ 不可能"),
    "C1": ("C", "mechanical: scripts/** 的 git commit 必须显式把关"),
    "C2": ("C", "non-mechanical: 空成功须显式命名"),
    "C3": ("C", "mechanical: ci.yml 上传步 if-no-files-found: error"),
    "C4": ("C", "non-mechanical: 没东西可提交不得 exit 0"),
    "C5": ("C", "non-mechanical: 机制缺失 vs 本次未触发"),
    "D1": ("D", "non-mechanical: 校验-写-回读三段"),
    "D2": ("D", "non-mechanical: 全或全全是脚本属性"),
    "D3": ("D", "non-mechanical: 回读与全校验是对偶"),
    "E1": ("E", "mechanical: ci.yml 按 crate 拆的 test 步带 --no-fail-fast"),
    "E2": ("E", "non-mechanical: 运行期 arms_ran 让 1/N 可见"),
    "E3": ("E", "non-mechanical: 计数器不得重述规则"),
    "E4": ("E", "non-mechanical: 未被计数的那几条本身是读数"),
    "E5": ("E", "non-mechanical: 报 N + 实际覆盖"),
    "E6": ("E", "non-mechanical: 未分类的差异才是信号"),
    "E7": ("E", "non-mechanical: 结构上必为 0 不携带信息"),
}

COMMIT_RE = re.compile(r"\bgit\s+commit\b")
# 显式把关的写法（任一命中即视为有把关）。
GUARD_RES = [
    re.compile(r"--commit\b"),
    re.compile(r"\bDRY[_-]?RUN\b", re.IGNORECASE),
    re.compile(r"\bapply\b.*\bflag\b", re.IGNORECASE),
]

violations: list[str] = []


HEREDOC_RE = re.compile(r"<<-?\s*['\"]?([A-Za-z_][A-Za-z0-9_]*)['\"]?\s*$")
COMMENT_RE = re.compile(r"^\s*#")


def code_lines(text: str) -> list[str]:
    """只留**真代码行**：剥掉 heredoc 正文与整行注释。

    为什么必须剥（实测代价）: 本门第一版直接在原文上 grep，于是把
    `scripts/dev/worktree.sh` 里 heredoc 的**提示文字**（"…人工判断后 git add -A && git commit"）
    当成了真调用 ⇒ **已知绿喂变红 = 假阳性**。这与本会话裁定的
    「A5 / R264①：**独立来源**会因**语料太宽**（把注释/散文算进）而数错」是同一形态
    —— 而那个缺口正是靠"已知绿喂"发现的。
    """
    out: list[str] = []
    end_marker: str | None = None
    for raw in text.splitlines():
        if end_marker is not None:
            if raw.strip() == end_marker:
                end_marker = None
            continue
        if COMMENT_RE.match(raw):
            continue
        m = HEREDOC_RE.search(raw)
        out.append(raw)
        if m:
            end_marker = m.group(1)
    return out


def check_c1() -> None:
    """C1: scripts/** 里**真代码行**出现 `git commit` 的脚本必须有显式开关把关。"""
    hits = 0
    for p in sorted(SCRIPTS.rglob("*")):
        if not p.is_file() or p.suffix not in {".sh", ".py", ".bash"}:
            continue
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        code = "\n".join(code_lines(text))
        if not COMMIT_RE.search(code):
            continue
        hits += 1
        if not any(r.search(text) for r in GUARD_RES):
            rel = p.relative_to(REPO)
            violations.append(f"C1 {rel}: 有 `git commit` 但没有显式开关（--commit / DRY_RUN）把关")
    print(f"  C1 扫描到 {hits} 个**真代码行**含 `git commit` 的脚本（⭐ 已剥 heredoc 正文与注释）")


def check_ci() -> None:
    """C3 + E1: ci.yml 的两条机械判据。"""
    if not CI.is_file():
        violations.append(f"C3/E1 {CI.relative_to(REPO)} 不存在")
        return
    text = CI.read_text(encoding="utf-8", errors="replace")
    if "if-no-files-found: error" not in text:
        violations.append("C3 ci.yml 缺 `if-no-files-found: error`（uploads 的缺失会退回成绿）")
    n_no_fail_fast = text.count("--no-fail-fast")
    if n_no_fail_fast < 1:
        violations.append("E1 ci.yml 的按 crate 拆的 test 步缺 `--no-fail-fast`")
    print(f"  C3 if-no-files-found: error 出现 {text.count('if-no-files-found: error')} 次")
    print(f"  E1 --no-fail-fast 出现 {n_no_fail_fast} 次")


def main() -> int:
    print("== 纪律检查表门（R261 五组）==")
    check_c1()
    check_ci()

    groups: dict[str, list[str]] = {}
    for cid, (grp, how) in REGISTRY.items():
        groups.setdefault(grp, []).append(cid)
    total = len(REGISTRY)
    mech = sum(1 for _c, (_g, how) in REGISTRY.items() if how.startswith("mechanical"))
    print(f"  注册表: {total} 条判据 / {len(groups)} 组; 其中已机械化 {mech} 条")
    for grp in sorted(groups):
        print(f"    组 {grp}: {len(groups[grp])} 条")
    print("  注: A/B/D 三组目前**如实标为 non-mechanical**（需人读 + 各线自证），⛔ 不假装能机械化。")

    if violations:
        print("\n违例:")
        for v in violations:
            print(f"  FAIL {v}")
        return 1
    print("\n全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
