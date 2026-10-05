#!/usr/bin/env bash
# 远程分支卫生: **已合并进 main 的工作线分支不应继续留在 origin 上**。
#
# 为什么要机械检查: 本仓库的口径是"远程只有 main + website + 正在跑的工作线"。
# 实测漂移: 第 12 轮发现 origin 上积了 **12 条已合并**的 `line/*` 分支 ——
# 而账本与状态表当时写着"远程只剩 main 与 website"。**人写的口径会漂移, 机器写的不会。**
#
# 历史并没有丢: 每条线退役时会留下 `line-archive/<line>` 标签（本地 + 已推送）。
#
# 用法: `bash scripts/dev/branch-hygiene.sh`（只读, 不改任何东西）
#   退出码 0 = 干净; 1 = 有已合并的远程分支应当删除（并打印删除命令）; 2 = 无法判定（网络/仓库问题）。
set -uo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "不在 git 仓库里" >&2; exit 2; }
cd "$repo_root"

git fetch --prune --quiet origin 2>/dev/null || { echo "无法 fetch origin（离线？）" >&2; exit 2; }

stale=()
for branch in $(git ls-remote --heads origin | sed 's|.*refs/heads/||' | grep '^line/' || true); do
  # 已合并进 main ⇒ 它已完成使命, 远程不该再留着（归档标签已保存历史）。
  if git merge-base --is-ancestor "origin/$branch" main 2>/dev/null; then
    stale+=("$branch")
  fi
done

if [[ ${#stale[@]} -eq 0 ]]; then
  echo "[ok] 远程分支卫生: origin 上没有【已合并却仍留着】的工作线分支"
  exit 0
fi

echo "以下工作线分支**已合并进 main**, 但仍留在 origin（应当删除, 历史由 line-archive/* 标签保存）:" >&2
for branch in ${stale[@]+"${stale[@]}"}; do
  echo "  $branch" >&2
done
echo >&2
echo "删除命令:" >&2
echo "  git push origin --delete ${stale[*]:-}" >&2
exit 1
