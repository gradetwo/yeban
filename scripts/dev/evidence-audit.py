#!/usr/bin/env python3
"""证据审计：**账本里被引用的 CI run id 是否真的存在、结论是什么**。

为什么需要它: 本仓库的口径是"**每一个结论都要有可复跑的判决**"（`AGENTS.md` §5.5）。
但"引用了一个 run id"本身**不等于**"那个判决支持你说的话" —— 三种典型失效:
① 引用了**不存在**的 run（记错/编造）; ② 引用了**失败或已取消**的 run 却当成绿（L23/L26 那一族）;
③ 引用的 run 属于**另一条分支/另一次提交**（判决归属错误）。

本脚本机械化 ① 与"结论是什么"这两半（② 的"措辞是否诚实"仍需人读，脚本会把非 success 的全部列出来供复核）。

用法: `python3 scripts/dev/evidence-audit.py`（只读; 需要 `gh` 或网络）。
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RUN_RE = re.compile(r"\brun\s+`?(\d{8,})`?")
DOCS = [REPO / "docs/DEVELOPMENT_LEDGER.md", *sorted((REPO / "docs/ledger").glob("*.md"))]
REPO_SLUG = "gradetwo/yeban"


def cited_runs() -> dict[str, list[str]]:
    """run id -> 引用它的文件（去重）。"""
    found: dict[str, list[str]] = {}
    for path in DOCS:
        if not path.is_file():
            continue
        for match in RUN_RE.finditer(path.read_text(encoding="utf-8")):
            ident = match.group(1)
            where = str(path.relative_to(REPO))
            found.setdefault(ident, [])
            if where not in found[ident]:
                found[ident].append(where)
    return found


def run_info(ident: str) -> dict | None:
    out = subprocess.run(
        ["gh", "api", f"repos/{REPO_SLUG}/actions/runs/{ident}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if out.returncode != 0 or not out.stdout.strip():
        return None
    try:
        data = json.loads(out.stdout)
    except json.JSONDecodeError:
        return None
    return {
        "conclusion": data.get("conclusion"),
        "branch": data.get("head_branch"),
        "name": data.get("name"),
        "sha": (data.get("head_sha") or "")[:7],
    }


def gh_available() -> bool:
    """`gh` 是否**已认证**。未认证时它仍然存在, 但每个请求都会失败 —— 那属于"无法判定", 不是"证据不存在"。

    实测教训: 我首次派发手动档 `inventory` 时没有给 token, 于是 75 个 run id 全报"不可读",
    整步红, **后面的检查（含新加的 cargo vendor）一个都没跑到**。所以这里必须先分清
    "没判定" 与 "判失败" —— 与 `check_vendor.sh` 里那条修正是同一条纪律。
    """
    out = subprocess.run(["gh", "auth", "status"], capture_output=True, text=True, check=False)
    return out.returncode == 0


def main() -> int:
    if not gh_available():
        print(
            "[unknown] `gh` 未认证 —— 本环境无法核对被引用的 run id（**不是**失败）。\n"
            "          CI 里请给该步骤 `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}`; 本地请 `gh auth login`。"
        )
        return 2
    runs = cited_runs()
    if not runs:
        print("[ok] 证据审计: 文档里没有引用任何 run id")
        return 0
    missing: list[str] = []
    non_success: list[tuple[str, dict, list[str]]] = []
    ok = 0
    for ident in sorted(runs):
        info = run_info(ident)
        if info is None:
            missing.append(ident)
            continue
        if info["conclusion"] == "success":
            ok += 1
        else:
            non_success.append((ident, info, runs[ident]))
    print(f"被引用的 run id: {len(runs)} 个（success {ok} / 非 success {len(non_success)} / 不可读 {len(missing)}）")
    for ident, info, where in non_success:
        print(
            f"  ⚠ {ident} → {info['conclusion']} | {info['branch']} | {info['sha']} | {info['name']}"
            f"\n      引用处: {', '.join(where[:3])}"
            f"\n      ⇒ 请**人工确认**它在文中被如实表述为失败/取消，而不是被当成绿（L23/L26）"
        )
    for ident in missing:
        print(f"  ✗ {ident} → 无法读取（不存在? 权限? 网络?）  引用处: {', '.join(runs[ident][:3])}")
    if missing:
        print("\n有**读不到**的 run id —— 那等同于【没有证据】（AGENTS.md §5.5）。")
        return 1
    print("\n[ok] 所有被引用的 run id 都存在; 非 success 的那些已逐条列出供人工核对措辞。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
