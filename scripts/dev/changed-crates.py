#!/usr/bin/env python3
"""CI 计划器: 由「改了哪些文件」推导出「必须跑哪些 crate 的门禁」。

对应 SKILL 规则 6 —— 受影响集合要用**搜索**得出, 不能凭记忆:
每条工作线只跑自己碰到的那一族检查, 而不是全量重跑, 这样多线并行才快。
同时这也防止「改了一个 crate 却漏跑它的下游」。

用法:
    python3 scripts/dev/changed-crates.py --base origin/main --head HEAD
    python3 scripts/dev/changed-crates.py --base "$BASE" --head HEAD --github-output
输出 (stdout, JSON):
    {"crates": ["yeban-model", ...], "workspace_wide": false, "reason": "..."}

`--github-output` 模式下会把 `crates` / `workspace_wide` / `reason` 写进 $GITHUB_OUTPUT,
并把人类可读的计划写进 $GITHUB_STEP_SUMMARY。把这段逻辑放在脚本里而不是
workflow 的 YAML heredoc 里, 是为了让它可以**在本机被测试** —— 藏在 YAML 里的逻辑
只能靠推送后读判决, 那是 SKILL 规则 3 明确否定的做法。

保守策略: 任何不确定的情况 (git 失败 / 根文件改动 / 无法映射) 一律返回
workspace_wide = true, 让 CI 退回全量 —— 宁可多跑, 不可漏跑。
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

#: 改动这些文件会影响整个工作区
ROOT_TRIGGERS = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "deny.toml",
    ".cargo/",
    "scripts/",
    "schemas/",
    ".github/",
)

MEMBER_ROOTS = ("crates/", "spikes/")


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=REPO,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def changed_files(base: str, head: str) -> list[str]:
    out = git("diff", "--name-only", f"{base}...{head}")
    files = [line.strip() for line in out.splitlines() if line.strip()]
    if files:
        return files
    # 回退: 三点点号在浅克隆/无共同祖先时会失败或为空, 用两点号再试一次
    out = git("diff", "--name-only", base, head)
    return [line.strip() for line in out.splitlines() if line.strip()]


def all_members() -> list[str]:
    members: list[str] = []
    for parent in ("crates", "spikes"):
        base = REPO / parent
        if base.is_dir():
            members.extend(sorted(d.name for d in base.iterdir() if (d / "Cargo.toml").is_file()))
    return members


def dependents_of(crate: str, members: list[str]) -> set[str]:
    """找出直接依赖 `crate` 的成员 (workspace 内部路径依赖)。"""
    out: set[str] = set()
    for other in members:
        for parent in ("crates", "spikes"):
            manifest = REPO / parent / other / "Cargo.toml"
            if not manifest.is_file():
                continue
            text = manifest.read_text(encoding="utf-8")
            if f'"{parent}/{crate}"' in text or f"path = \"{parent}/{crate}\"" in text:
                out.add(other)
    return out


#: 空集合的哨兵值。GitHub Actions 的 matrix 不接受空数组, 用一个不可能存在的
#: 名字占位, 由 workflow 用 `if` 跳过 —— 比让 job 神秘消失要好排查。
EMPTY_SENTINEL = "none"


def emit(result: dict, github_output: bool) -> None:
    print(json.dumps(result, ensure_ascii=False))
    if not github_output:
        return
    crates = result["crates"] or [EMPTY_SENTINEL]
    out_path = os.environ.get("GITHUB_OUTPUT")
    if out_path:
        with open(out_path, "a", encoding="utf-8") as fh:
            fh.write(f"crates={json.dumps(crates)}\n")
            fh.write(f"workspace_wide={'true' if result['workspace_wide'] else 'false'}\n")
            fh.write(f"reason={result['reason']}\n")
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        listed = ", ".join(f"`{c}`" for c in crates)
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write("### 计划 (plan)\n\n")
            fh.write(f"- 全量门禁: `{result['workspace_wide']}`\n")
            fh.write(f"- 理由: {result['reason']}\n")
            fh.write(f"- 受影响 crate: {listed}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description="推导受影响 crate 集合")
    parser.add_argument("--base", required=True, help="基线 ref (例如 origin/main)")
    parser.add_argument("--head", default="HEAD", help="目标 ref (默认 HEAD)")
    parser.add_argument(
        "--github-output",
        action="store_true",
        help="把结果写进 $GITHUB_OUTPUT / $GITHUB_STEP_SUMMARY (CI 用)",
    )
    parser.add_argument(
        "--force-full",
        action="store_true",
        help="不做差异推导, 直接返回全部成员 (手动档要真跑全量时用)",
    )
    args = parser.parse_args()
    gho = args.github_output

    members = all_members()

    if args.force_full:
        # 真实教训: 手动档最初用 `--base HEAD~1` 想"强制全量", 那只把差异缩到最后一个提交,
        # 结果 plan 推导出"无受影响 crate", rust 矩阵**被跳过**却报 success ——
        # 一个声称跑全量、实际什么都没跑的手动档, 比没有这个档位更危险。
        emit(
            {
                "crates": members,
                "workspace_wide": True,
                "reason": "--force-full: 跳过差异推导, 强制全部成员",
            },
            gho,
        )
        return 0

    try:
        files = changed_files(args.base, args.head)
    except subprocess.CalledProcessError as exc:
        emit(
            {
                "crates": members,
                "workspace_wide": True,
                "reason": f"git diff 失败, 保守回退全量: {exc}",
            },
            gho,
        )
        return 0

    if not files:
        emit(
            {
                "crates": members,
                "workspace_wide": True,
                "reason": "未检测到差异 (可能是首次推送), 保守全量",
            },
            gho,
        )
        return 0

    if any(f == t or f.startswith(t) for f in files for t in ROOT_TRIGGERS):
        emit(
            {
                "crates": members,
                "workspace_wide": True,
                "reason": "根级文件 (Cargo.toml/lock/工具链/脚本/schema/CI) 被改动",
            },
            gho,
        )
        return 0

    touched: set[str] = set()
    for f in files:
        for root in MEMBER_ROOTS:
            if f.startswith(root):
                name = f[len(root) :].split("/", 1)[0]
                if name in members:
                    touched.add(name)

    if not touched:
        emit(
            {
                "crates": [],
                "workspace_wide": False,
                "reason": "改动不落在任何成员 crate 内 (纯文档/资产), 且根级触发器未命中",
            },
            gho,
        )
        return 0

    # 传递闭包: 依赖被改 crate 的成员也必须重跑
    closure = set(touched)
    frontier = set(touched)
    while frontier:
        nxt: set[str] = set()
        for crate in frontier:
            nxt |= dependents_of(crate, members) - closure
        closure |= nxt
        frontier = nxt

    emit(
        {
            "crates": sorted(closure),
            "workspace_wide": False,
            "reason": f"{len(files)} 个文件改动, 命中 {len(touched)} 个 crate, 含下游共 {len(closure)} 个",
        },
        gho,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
