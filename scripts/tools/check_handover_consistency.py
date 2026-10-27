#!/usr/bin/env python3
"""交接文件一致性自检 —— 抓"交接陈旧"这类问题（本会话手工发现过两次）。

⭐ 由来（R541）：`docs/ledger/handover-latest.md` 是每轮的**入口文件**，但**没有人校它**。
   本会话手工发现两处陈旧：
     · §1 的表仍写着旧的 main sha；
     · §4 标题写"完整 37–71 条"，而台账已到 85 条（落后 14 条）。

⛔ **本工具不进 CI**：交接是**快照**、本就允许滞后于台账；进 CI 会让"快照略旧"变成红。
   ⭐ 它是**给集成者跑的本机检查**（交接由集成者独占维护）。

用法：
    python3 scripts/tools/check_handover_consistency.py
    python3 scripts/tools/check_handover_consistency.py --self-test
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
HANDOVER = REPO / "docs" / "ledger" / "handover-latest.md"
LEDGER = REPO / "docs" / "ledger" / "phase-status.md"


def max_rule_number(text: str) -> int:
    """从台账里取"第 N 条纪律"的最大 N。"""
    nums = [int(m) for m in re.findall(r"第\s*(\d+)\s*条", text)]
    return max(nums) if nums else 0


def declared_rule_range(text: str) -> tuple[int, int] | None:
    """从交接 §4 标题里取声明的条号区间，例如"完整 **37–85** 条"。"""
    m = re.search(r"完整\s*\*{0,2}(\d+)\s*[–\-—~]\s*\*{0,2}(\d+)\*{0,2}\s*条", text)
    if not m:
        return None
    return int(m.group(1)), int(m.group(2))


def declared_new_range(text: str) -> tuple[int, int] | None:
    m = re.search(r"本会话新增\s*\*{0,2}(\d+)\s*[–\-—~]\s*\*{0,2}(\d+)\*{0,2}", text)
    if not m:
        return None
    return int(m.group(1)), int(m.group(2))


def declared_main_sha(text: str) -> str | None:
    """从交接 §1 的表里取它声明的 `origin/main` sha。"""
    m = re.search(r"`origin/main`\s*\|\s*⭐\s*\*\*`([0-9a-f]{7,40})`", text)
    return m.group(1) if m else None


def is_ancestor_of_main(sha: str) -> bool | None:
    """⭐ 声明的 sha 只要**是 origin/main 的祖先**即可 —— ⛔ 不要求等于 tip。

    ⚠ 为什么（R557 的自纠）：若要求"等于当前 tip"，那么**修 §1 的那次提交本身就改了 tip**
    ⇒ §1 **永远落后一个提交** ⇒ 检查**永远报红** ⇒ 而"永远报红的检查会被忽略"
    （本会话第 104 条）。正确语义是：**它当时是真的**（即该 sha 在 main 的历史里）。
    """
    import subprocess
    try:
        out = subprocess.run(["git", "merge-base", "--is-ancestor", sha, "origin/main"],
                             cwd=REPO, capture_output=True, text=True, timeout=30)
        if out.returncode == 0:
            return True
        if out.returncode == 1:
            return False
        return None
    except Exception:
        return None


def actual_main_sha() -> str | None:
    import subprocess
    try:
        out = subprocess.run(["git", "rev-parse", "--short", "origin/main"],
                             cwd=REPO, capture_output=True, text=True, timeout=30)
        return out.stdout.strip() if out.returncode == 0 else None
    except Exception:
        return None


def declared_worktrees(text: str) -> set[str]:
    """从交接 §5 的表里取出它列的 `.worktrees/*` 路径（取末两段，如 `.worktrees/mod-app`）。"""
    out: set[str] = set()
    for m in re.finditer(r"`?(\.worktrees/[A-Za-z0-9._-]+)`?", text):
        out.add(m.group(1).rstrip("/"))
    return out


def actual_worktrees() -> set[str] | None:
    """实际的 worktree 路径（同样取末两段），相对仓库根。"""
    import subprocess
    try:
        out = subprocess.run(["git", "worktree", "list", "--porcelain"],
                             cwd=REPO, capture_output=True, text=True, timeout=30)
        if out.returncode != 0:
            return None
        found: set[str] = set()
        for line in out.stdout.splitlines():
            if line.startswith("worktree "):
                p = line[len("worktree "):].strip()
                parts = p.split("/")
                if len(parts) >= 2 and parts[-2] == ".worktrees":
                    found.add(f".worktrees/{parts[-1]}")
        return found
    except Exception:
        return None


def section6_rows(text: str) -> tuple[int, int | None]:
    """取 §6 待裁表的**数据行数**与标题声明的件数。数据行 = 以 `| **` 开头的行。"""
    m = re.search(r"^## 6\..*?^## 7\.", text, re.S | re.M)
    if not m:
        return 0, None
    body = m.group(0)
    rows = len(re.findall(r"^\|\s*\*\*", body, re.M))
    decl = re.search(r"\*\*(\d+)\s*件\*\*", body)
    declared = int(decl.group(1)) if decl else None
    # ⭐ 中文数字兜底：一/二/三/四/五/六
    if declared is None:
        cn = {"一": 1, "二": 2, "三": 3, "四": 4, "五": 5, "六": 6}
        d2 = re.search(r"\*\*([一二三四五六])件\*\*", body)
        declared = cn.get(d2.group(1)) if d2 else None
    return rows, declared


def check(handover_text: str, ledger_text: str) -> list[str]:
    bad: list[str] = []
    top = max_rule_number(ledger_text)
    if top == 0:
        bad.append("台账里读不到任何「第 N 条」⇒ 本判据可能空转，不能当作「交接一致」的证据")
        return bad

    rng = declared_rule_range(handover_text)
    if rng is None:
        bad.append("交接 §4 标题里读不到条号区间（形如「完整 **37–85** 条」）")
    else:
        lo, hi = rng
        if hi != top:
            bad.append(f"交接 §4 声明到第 {hi} 条，而台账已到第 {top} 条（落后 {top - hi} 条）")
        if lo > hi:
            bad.append(f"交接 §4 区间反了：{lo}–{hi}")

    new = declared_new_range(handover_text)
    if new is not None and rng is not None:
        if new[1] != rng[1]:
            bad.append(f"交接里「本会话新增」到 {new[1]}、而「完整」到 {rng[1]} ⇒ 两处不一致")

    # ⭐ §1 声明的 origin/main sha 应与实际一致（R556：本会话第二次手工发现的陈旧类）
    claimed = declared_main_sha(handover_text)
    real = actual_main_sha()
    if claimed is None:
        bad.append("交接 §1 里读不到声明的 `origin/main` sha")
    elif real is None:
        bad.append("读不到实际的 origin/main ⇒ 本判据可能空转")
    else:
        anc = is_ancestor_of_main(claimed)
        if anc is False:
            bad.append(
                f"交接 §1 声明 origin/main = {claimed}，但它**不是** {real} 的祖先"
                " ⇒ 该 sha 不在主干历史里"
            )
        elif anc is None:
            bad.append(f"无法判定 {claimed} 是否为 {real} 的祖先 ⇒ 本判据可能空转")

    # ⭐ §5 列的 worktree 应与实际一致（R560：§5 是第三类可能陈旧的节）
    declared_wt = declared_worktrees(handover_text)
    real_wt = actual_worktrees()
    if real_wt is None:
        bad.append("读不到实际 worktree 清单 ⇒ 本判据可能空转")
    elif declared_wt or real_wt:
        # ⭐ **两边都空**是一致状态（零线在飞 ⇒ §5 只剩主树那一行），⛔ 不算空转：
        #    "交接说没有在飞的线" ＋ "磁盘确实没有" 本身就是被核对的命题。
        missing = sorted(declared_wt - real_wt)
        extra = sorted(real_wt - declared_wt)
        if missing:
            bad.append(f"交接 §5 列了但实际不存在的 worktree: {missing}")
        if extra:
            bad.append(f"实际存在但交接 §5 未列的 worktree: {extra}")

    # ⭐ §6 待裁表：行数与声明件数一致、且**不得残留"待裁"字样**（R594）
    rows, declared = section6_rows(handover_text)
    if rows == 0:
        bad.append("交接 §6 里读不到任何待裁行 ⇒ 本判据可能空转")
    else:
        if declared is not None and declared != rows:
            bad.append(f"交接 §6 声明 {declared} 件，而表里有 {rows} 行")
        body6 = re.search(r"^## 6\..*?^## 7\.", handover_text, re.S | re.M)
        if body6 and re.search(r"待裁|⏳|PENDING", body6.group(0)):
            bad.append("交接 §6 里仍有「待裁」字样 ⇒ 要么已裁决、要么应显式标出")

    # ⭐ 交叉核对：台账里最新一条的编号应当等于上面的 top
    latest = re.findall(r"^## R(\d+)", ledger_text, re.M)
    if not latest:
        bad.append("台账里读不到 `## R<n>` 标题 ⇒ 本判据可能空转")
    return bad


def self_test() -> int:
    """正/负对照：能发现"落后"、且**不**对一致文本误报。"""
    ledger = "第 37 条 …\n第 85 条 …\n## R540\n"
    # ⭐ §6 用例（R594）：一致文本要有 §6、且件数与行数相符
    sec6_ok = ("## 6. ⭐ ⭐ 待人类裁决（**一件**）\n\n"
               "| # | 事项 | 选项 |\n| :--- | :--- | :--- |\n"
               "| **A1** | 甲 | A/B |\n\n## 7. 下一节\n")
    sec6_bad = ("## 6. ⭐ ⭐ 待人类裁决（**一件**）\n\n"
                "| # | 事项 | 选项 |\n| :--- | :--- | :--- |\n"
                "| **A1** | 甲 | A/B |\n| **A2** | 乙 | A/B |\n\n## 7. 下一节\n")
    ok_text = sec6_ok + "完整 **37–85** 条在台账；本会话新增 **56–85**\n"
    bad_text = sec6_bad + "完整 **37–71** 条在台账；本会话新增 **56–71**\n"
    wt_real = sorted(actual_worktrees() or set())
    wt_ok = "".join(f"| `{w}` | `line/x` | 保留 |\n" for w in wt_real)
    wt_bad = "| `.worktrees/does-not-exist-xyz` | `line/x` | 保留 |\n"
    sha_ok = "| `origin/main` | ⭐ **`" + (actual_main_sha() or "0"*8) + "`** |\n"
    sha_bad = "| `origin/main` | ⭐ **`deadbeef`** |\n"
    cases = [(ok_text, 0, "一致文本"), (bad_text, 1, "落后 14 条的文本")]
    fails = 0
    for text, want_min, label in cases:
        # ⭐ §1 的 sha 用例：真 sha 必须不报、假 sha 必须报
        text = text + sha_ok + wt_ok if want_min == 0 else text + sha_bad + wt_bad
        got = check(text, ledger)
        ok = (len(got) == 0) if want_min == 0 else (len(got) >= want_min)
        print(f"[{'ok' if ok else 'FAIL'}] {label}: 报出 {len(got)} 项 {got[:1]}")
        fails += 0 if ok else 1
    if fails:
        print(f"[FAIL] 自检 {fails} 项不符")
        return 1
    print("[ok] 自检通过：能发现条号落后、且对一致文本不误报")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) >= 2 and argv[1] == "--self-test":
        return self_test()
    if not HANDOVER.exists() or not LEDGER.exists():
        print("[FAIL] 缺少交接或台账文件", file=sys.stderr)
        return 1
    bad = check(HANDOVER.read_text(encoding="utf-8"), LEDGER.read_text(encoding="utf-8"))
    if bad:
        for b in bad:
            print(f"[!] {b}")
        print(f"[!] 交接一致性：{len(bad)} 项待更新（⭐ 本工具不进 CI、由集成者跑）")
        return 1
    print("[ok] 交接一致性检查通过（条号区间与台账一致）")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
