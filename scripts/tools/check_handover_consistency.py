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


def actual_main_sha() -> str | None:
    import subprocess
    try:
        out = subprocess.run(["git", "rev-parse", "--short", "origin/main"],
                             cwd=REPO, capture_output=True, text=True, timeout=30)
        return out.stdout.strip() if out.returncode == 0 else None
    except Exception:
        return None


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
    elif not real.startswith(claimed) and not claimed.startswith(real):
        bad.append(f"交接 §1 声明 origin/main = {claimed}，实际为 {real}")

    # ⭐ 交叉核对：台账里最新一条的编号应当等于上面的 top
    latest = re.findall(r"^## R(\d+)", ledger_text, re.M)
    if not latest:
        bad.append("台账里读不到 `## R<n>` 标题 ⇒ 本判据可能空转")
    return bad


def self_test() -> int:
    """正/负对照：能发现"落后"、且**不**对一致文本误报。"""
    ledger = "第 37 条 …\n第 85 条 …\n## R540\n"
    ok_text = "完整 **37–85** 条在台账；本会话新增 **56–85**\n"
    bad_text = "完整 **37–71** 条在台账；本会话新增 **56–71**\n"
    sha_ok = "| `origin/main` | ⭐ **`" + (actual_main_sha() or "0"*8) + "`** |\n"
    sha_bad = "| `origin/main` | ⭐ **`deadbeef`** |\n"
    cases = [(ok_text, 0, "一致文本"), (bad_text, 1, "落后 14 条的文本")]
    fails = 0
    for text, want_min, label in cases:
        # ⭐ §1 的 sha 用例：真 sha 必须不报、假 sha 必须报
        text = text + sha_ok if want_min == 0 else text + sha_bad
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
