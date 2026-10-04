#!/usr/bin/env bash
# 多线并行开发的工作树 (worktree) 生命周期工具。
#
# 纪律 (docs/skills/yeban-dev-workflow/SKILL.md「Fast multi-line iteration」):
#   - 一条工作线 = 一个 worktree = 一个分支, 两条线**永不**共用同一棵树;
#   - 一个文件只能有一个写者; 合并才是速度流失的地方;
#   - 工作线短命, 通过同一套门禁落地;
#   - 被取代的工作线要**显式废弃** (打标签/打包/删分支), 陈旧分支是负债不是备份。
#
# 用法:
#   scripts/dev/worktree.sh add  <line> [base]     新建 line/<line> 分支与工作树
#   scripts/dev/worktree.sh list                   列出全部工作线
#   scripts/dev/worktree.sh path <line>            打印工作树路径
#   scripts/dev/worktree.sh rm   <line> [--purge]  移除工作树 (--purge 连同分支一起删)
#   scripts/dev/worktree.sh land <line>            在 main 上合并该线 (--no-ff, 保留线史)
#
# 工作树统一放在仓库内 `.worktrees/` (已 gitignore), 避免污染上层目录。
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WT_ROOT="$REPO/.worktrees"
MAIN_BRANCH="${YEBAN_MAIN_BRANCH:-main}"

die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
info() { printf '\033[1m%s\033[0m\n' "$*"; }

require_clean_tree() {
  if [[ -n "$(git -C "$REPO" status --porcelain)" ]]; then
    die "工作树不干净, 先提交或 stash (多线开发时脏树是事故来源)"
  fi
}

cmd_add() {
  local line="${1:-}"; local base="${2:-$MAIN_BRANCH}"
  [[ -n "$line" ]] || die "用法: worktree.sh add <line> [base]"
  [[ "$line" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || die "线名只允许小写字母/数字/._- : '$line'"
  local branch="line/$line" path="$WT_ROOT/$line"
  [[ -e "$path" ]] && die "路径已存在: $path"
  git -C "$REPO" show-ref --verify --quiet "refs/heads/$branch" && die "分支已存在: $branch"

  info "创建工作线 $branch (基于 $base) -> $path"
  git -C "$REPO" worktree add -b "$branch" "$path" "$base"
  cat <<EOF

工作线已就绪: $path
  进入:  cd $path
  门禁:  bash scripts/dev/cargo-local.sh test -p <crate>     # 轻量 crate
         bash scripts/gates/run-gates.sh light               # 格式 + 红线守卫
  推送:  git push -u origin $branch                            # CI 自动触发
  落地:  scripts/dev/worktree.sh land $line
EOF
}

cmd_list() {
  info "工作树:"
  git -C "$REPO" worktree list
  echo
  info "活跃工作线分支 (line/*):"
  git -C "$REPO" branch --list 'line/*' || true
}

cmd_path() {
  local line="${1:-}"; [[ -n "$line" ]] || die "用法: worktree.sh path <line>"
  echo "$WT_ROOT/$line"
}

cmd_rm() {
  local line="${1:-}"; local purge="${2:-}"
  [[ -n "$line" ]] || die "用法: worktree.sh rm <line> [--purge]"
  local branch="line/$line" path="$WT_ROOT/$line"
  info "移除工作树 $path"
  git -C "$REPO" worktree remove "$path" ${purge:+--force} 2>/dev/null || git -C "$REPO" worktree remove --force "$path"
  git -C "$REPO" worktree prune
  if [[ "$purge" == "--purge" ]]; then
    info "删除分支 $branch (显式废弃)"
    git -C "$REPO" branch -D "$branch"
  else
    info "分支 $branch 保留 (未合并的工作不应静默消失)"
  fi
}

cmd_land() {
  local line="${1:-}"; [[ -n "$line" ]] || die "用法: worktree.sh land <line>"
  local branch="line/$line"
  require_clean_tree
  git -C "$REPO" show-ref --verify --quiet "refs/heads/$branch" || die "分支不存在: $branch"
  info "把 $branch 合并进 $MAIN_BRANCH (--no-ff, 保留工作线历史)"
  git -C "$REPO" checkout "$MAIN_BRANCH"
  if ! git -C "$REPO" merge --no-ff "$branch" -m "merge($line): 工作线落地"; then
    cat >&2 <<EOF

合并未完成。**不要手工合并生成物** —— 按 docs/DEV_WORKFLOW.md「生成物冲突」处置:

  1. 冲突文件若是 Cargo.lock:       git checkout --ours Cargo.lock && cargo metadata --format-version 1 >/dev/null
  2. 冲突文件若是 dependency-licenses.md: python3 scripts/gates/license_inventory.py
  3. 其余文件人工判断后 git add -A && git commit

原因: 这两个文件是从真实状态**生成**的产物, 手工合并必然产生与真实依赖图不符的内容。
EOF
    return 1
  fi
  post_merge_checks
}

# 合并后必须做的事 (把政策固化进工具, 而不是靠人记得):
#   1. 生成物 (Cargo.lock / 许可清单) 与合并后的真实状态必须自洽;
#   2. 本机能跑的轻量门禁必须重跑 —— 树变了, 之前的绿不作数。
post_merge_checks() {
  info "合并后自检 (生成物一致性 + 轻量门禁)"
  local failed=0

  if [[ -f "$REPO/Cargo.lock" ]]; then
    if (cd "$REPO" && cargo metadata --locked --format-version 1 >/dev/null 2>&1); then
      echo "  ok   cargo metadata --locked (锁与清单一致)"
    else
      echo "  FAIL Cargo.lock 与合并后的清单不一致 —— 请跑: cargo metadata --format-version 1 >/dev/null" >&2
      failed=1
    fi
  fi

  if [[ -f "$REPO/docs/ledger/dependency-licenses.md" ]]; then
    if (cd "$REPO" && python3 scripts/gates/license_inventory.py --check >/dev/null 2>&1); then
      echo "  ok   依赖许可清单与依赖图一致"
    else
      echo "  FAIL 许可清单已漂移 —— 请跑: python3 scripts/gates/license_inventory.py" >&2
      failed=1
    fi
  fi

  if [[ $failed -eq 0 ]]; then
    info "自检通过。下一步: 推送 main → 读 CI 判决 (scripts/dev/ci-verdict.sh); 未读的判决等于 pending。"
  else
    info "自检未通过 (见上面的 FAIL)。**先修再推** —— 推上去只会让 CI 也红一次。"
  fi
  return $failed
}

case "${1:-}" in
  add)  shift; cmd_add "$@" ;;
  list) cmd_list ;;
  path) shift; cmd_path "$@" ;;
  rm)   shift; cmd_rm "$@" ;;
  land) shift; cmd_land "$@" ;;
  *) cat >&2 <<EOF
用法: worktree.sh <add|list|path|rm|land> ...

  add  <line> [base]     新建 line/<line> 工作树
  list                   列出工作树与工作线分支
  path <line>            打印工作树路径
  rm   <line> [--purge]  移除工作树 (--purge 一并删分支)
  land <line>            合并进 $MAIN_BRANCH
EOF
     exit 2 ;;
esac
