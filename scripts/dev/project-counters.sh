#!/usr/bin/env bash
# 治理计数: 把账本/状态表里写着的**数字类事实**重新导出一遍。
#
# 为什么需要它: 我自己的规则是"**凡是写进口径的事实, 要么能被一条命令复核, 要么就别写成事实**"。
# 实测漂移过两次: ① 远程分支(账本写"只剩 main 与 website", 实际积了 12 条已合并的 line/*);
# ② 本脚本首次运行时抓到 归档标签 20→33、有真实实现的 crate 13→11、提交数早已不是那个数。
# ⇒ 快照里的数字都必须能被这条命令重新导出, 而不是靠"上次写的时候是对的"。
#
# 用法: `bash scripts/dev/project-counters.sh`（只读, 不改任何东西）
set -uo pipefail
repo_root="$(git rev-parse --show-toplevel)" || exit 2
cd "$repo_root"

scaffolds=0
real=0
real_loc=0
for dir in crates/*/; do
  loc=$(find "$dir/src" -name '*.rs' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
  loc=${loc:-0}
  if [ "$loc" -lt 200 ]; then
    scaffolds=$((scaffolds + 1))
  else
    real=$((real + 1))
    real_loc=$((real_loc + loc))
  fi
done

adr_max=$(grep -oE '^#+ D[0-9]+' docs/adr/ADR-0001-*.md 2>/dev/null | grep -oE '[0-9]+' | sort -n | tail -1)
lessons=$(grep -oE '^#+ L[0-9]+' docs/DEVELOPMENT_LEDGER.md 2>/dev/null | grep -oE 'L[0-9]+' | sort -u | wc -l | tr -d ' ')
tags=$(git tag --list 'line-archive/*' | wc -l | tr -d ' ')
commits=$(git rev-list --count HEAD)
guards=$(python3 scripts/guards/policy_check.py --list 2>/dev/null | wc -l | tr -d ' ')
ledgers=$(ls docs/ledger/*.md 2>/dev/null | wc -l | tr -d ' ')
worktrees=$(git worktree list 2>/dev/null | grep -c 'line/')

echo "crates-real=${real} crates-scaffold=${scaffolds} crates-real-src-lines=${real_loc}"
echo "adr-rulings=D1-D${adr_max} ledger-lessons=${lessons}"
echo "archive-tags=${tags} commits=${commits} guards=${guards} ledger-files=${ledgers} active-worktrees=${worktrees}"
if command -v git >/dev/null 2>&1; then
  remote=$(git ls-remote --heads origin 2>/dev/null | sed 's|.*refs/heads/||' | tr '\n' ' ')
  if [ -n "$remote" ]; then
    echo "remote-branches=${remote}"
  else
    echo "remote-branches=(无法读取 origin —— 离线?)"
  fi
fi
echo "--- 门禁状态分布（取自唯一事实源 docs/ledger/gate-status.md）---"
grep -oE '\*\*(已接线|部分|PENDING)' docs/ledger/gate-status.md | sort | uniq -c
