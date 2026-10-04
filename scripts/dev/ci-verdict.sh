#!/usr/bin/env bash
# 读取 GitHub Actions 的判决 (verdict)。
#
# 为什么需要它: docs/skills/yeban-dev-workflow/SKILL.md「Honesty rules」——
#   「本地绿不是绿; 尚未读取的判决是 pending」。
# 本机不编译重依赖, 所以真正的判决来自 CI; 这个脚本负责**把判决读回来**。
#
# 两条取数路径, 自动选择:
#   1. `gh` CLI 已登录  -> 走 gh (能直接拿到失败 job 的原始日志, 最省事);
#   2. 否则             -> 走公开 REST API (仓库是 public, 匿名即可读 run/job/step 状态;
#                          但原始日志下载需要 token, 因此日志功能会提示配置 GH_TOKEN)。
#
# 沙箱提示: `gh` 默认把缓存写在 ~/.cache/gh, 受限环境下不可写。脚本会在不可写时
# 自动把 XDG_CACHE_HOME 指到工作区内的 .cache/, 不改动用户的全局配置。
#
# 用法:
#   scripts/dev/ci-verdict.sh                  # 当前分支最新一次 run
#   scripts/dev/ci-verdict.sh website          # 指定分支
#   scripts/dev/ci-verdict.sh --logs <run-id>  # 拉取失败 job 日志
#   scripts/dev/ci-verdict.sh --watch [branch] # 轮询直到 run 结束
#   scripts/dev/ci-verdict.sh --list [n]       # 最近 n 次运行一览 (默认 10)
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO_SLUG="${YEBAN_REPO_SLUG:-gradetwo/yeban}"
API="https://api.github.com/repos/$REPO_SLUG"
AUTH=()
[[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]] && AUTH=(-H "Authorization: Bearer ${GH_TOKEN:-$GITHUB_TOKEN}")

die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
info() { printf '\033[1m%s\033[0m\n' "$*"; }
api() { curl -fsSL "${AUTH[@]}" -H "Accept: application/vnd.github+json" "$@"; }
current_branch() { git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD; }

has_gh() { command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; }

# gh 的缓存目录在受限沙箱里不可写; 只在不可写时才改, 避免污染正常环境。
# 关键: 落到**仓库之外的**工作区根 —— `gh run view --log` 会在缓存目录里写 run-log zip,
# 指到仓库内会让 `git add -A` 把它一起提交 (真实踩过一次)。
prepare_gh() {
  local cache="${XDG_CACHE_HOME:-$HOME/.cache}"
  if [[ ! -w "$cache" || ( -e "$cache/gh" && ! -w "$cache/gh" ) ]]; then
    export XDG_CACHE_HOME="$(cd "$REPO_ROOT/.." && pwd)/.cache"
    mkdir -p "$XDG_CACHE_HOME"
  fi
}

gh_list() {
  local limit="${1:-10}"
  prepare_gh
  gh run list --repo "$REPO_SLUG" --limit "$limit" \
    --json databaseId,displayTitle,status,conclusion,headBranch,event,createdAt,url \
    --template '{{range .}}{{printf "%-12v %-10v %-12v %-8v %v\n" .databaseId .conclusion .headBranch .event .displayTitle}}{{end}}'
}

gh_show() {
  local run_id="$1"
  prepare_gh
  gh run view "$run_id" --repo "$REPO_SLUG"
}

gh_logs() {
  local run_id="$1"
  prepare_gh
  gh run view "$run_id" --repo "$REPO_SLUG" --log-failed || gh run view "$run_id" --repo "$REPO_SLUG" --log
}

# 判决必须属于**分支 tip** 的 SHA —— 否则"读到的绿"是别的提交的绿。
#
# 为什么必须机械化: `ci.yml` 有 `concurrency.cancel-in-progress`, 于是**快速连续推送**会让
# 中间那些 SHA 的 run 被取消(甚至从列表里消失)。此时"某个 run 是绿的"与"我的代码被验证过"
# 是两件事 —— 我本人差点据此把一条未经 CI 验证的线合并进 main(第 4 轮, live-port)。
assert_verdict_matches_tip() {
  local branch="$1" sha="$2"
  local tip
  tip=$(git -C "$REPO_ROOT" rev-parse --verify --quiet "$branch" 2>/dev/null \
    || git -C "$REPO_ROOT" rev-parse --verify --quiet "origin/$branch" 2>/dev/null || echo "")
  [[ -n "$tip" ]] || return 0
  if [[ "${tip:0:8}" != "${sha:0:8}" ]]; then
    printf '\033[31m警告:\033[0m 这个判决属于 %s，而 %s 的 tip 是 %s\n' "${sha:0:8}" "$branch" "${tip:0:8}" >&2
    printf '  ⇒ 它\033[1m不能\033[0m当作 tip 的判决。大概率是 tip 的 run 仍排队/被 concurrency 取消。\n' >&2
    printf '  ⇒ 请等 tip 自己的 run，或重推一次触发。\033[1m未验证 ≠ 通过。\033[0m\n' >&2
    return 2
  fi
  return 0
}

# ---------------------------------------------------------------- REST path

latest_run_id() {
  local branch="$1"
  api "$API/actions/runs?branch=$branch&per_page=1" \
    | python3 -c 'import json,sys; runs=json.load(sys.stdin)["workflow_runs"]; print(runs[0]["id"] if runs else "")'
}

print_run() {
  local run_id="$1"
  api "$API/actions/runs/$run_id" | python3 -c '
import json, sys
r = json.load(sys.stdin)
print(f"run      : {r[\"id\"]}  ({r[\"name\"]})")
print(f"branch   : {r[\"head_branch\"]}  sha={r[\"head_sha\"][:8]}")
print(f"event    : {r[\"event\"]}")
print(f"status   : {r[\"status\"]}   conclusion: {r[\"conclusion\"]}")
print(f"url      : {r[\"html_url\"]}")
'
  echo
  api "$API/actions/runs/$run_id/jobs" | python3 -c '
import json, sys
jobs = json.load(sys.stdin)["jobs"]
if not jobs:
    print("(还没有 job)")
print(f"{\"job\":<34} {\"status\":<12} {\"conclusion\":<10} failed step")
print("-" * 96)
bad = 0
for j in jobs:
    failed = [s["name"] for s in j.get("steps", []) if s.get("conclusion") in ("failure", "timed_out")]
    bad += 1 if j.get("conclusion") == "failure" else 0
    print(f"{j[\"name\"][:33]:<34} {j[\"status\"]:<12} {str(j.get(\"conclusion\")):<10} {\"; \".join(failed) or \"-\"}")
print()
print(f"失败 job 数: {bad}")
sys.exit(1 if bad else 0)
'
}

rest_logs() {
  local run_id="$1"
  [[ ${#AUTH[@]} -gt 0 ]] || die "未登录 gh 且未提供 GH_TOKEN: 原始日志下载接口需要鉴权。请先 \`gh auth login\`。"
  local ids
  ids=$(api "$API/actions/runs/$run_id/jobs" | python3 -c '
import json,sys
for j in json.load(sys.stdin)["jobs"]:
    if j.get("conclusion") == "failure":
        print(j["id"])
')
  [[ -n "$ids" ]] || die "该 run 没有失败的 job"
  while read -r jid; do
    [[ -n "$jid" ]] || continue
    echo "===== job $jid 最后 120 行 ====="
    api "$API/actions/jobs/$jid/logs" | tail -120
  done <<< "$ids"
}

# -------------------------------------------------------------------- main

case "${1:-}" in
  --list)
    shift
    if has_gh; then gh_list "${1:-10}"; else
      api "$API/actions/runs?per_page=${1:-10}" | python3 -c '
import json,sys
for r in json.load(sys.stdin)["workflow_runs"]:
    print(f"{r[\"id\"]:<12} {str(r[\"conclusion\"]):<12} {r[\"head_branch\"]:<12} {r[\"event\"]:<8} {r[\"display_title\"][:60]}")
'
    fi
    ;;
  --logs)
    shift
    rid="${1:?用法: ci-verdict.sh --logs <run-id>}"
    if has_gh; then gh_logs "$rid"; else rest_logs "$rid"; fi
    ;;
  --watch)
    shift
    branch="${1:-$(current_branch)}"
    if has_gh; then
      prepare_gh
      while :; do
        rid=$(gh run list --repo "$REPO_SLUG" --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId')
        [[ -n "$rid" ]] || die "分支 $branch 上没有任何 workflow run"
        status=$(gh run view "$rid" --repo "$REPO_SLUG" --json status --jq .status)
        printf '\r[watch] run=%s status=%s   ' "$rid" "$status"
        [[ "$status" == "completed" ]] && { echo; break; }
        sleep 20
      done
      gh run view "$rid" --repo "$REPO_SLUG"
    else
      while :; do
        rid=$(latest_run_id "$branch")
        [[ -n "$rid" ]] || die "分支 $branch 上没有任何 workflow run"
        status=$(api "$API/actions/runs/$rid" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
        printf '\r[watch] run=%s status=%s   ' "$rid" "$status"
        [[ "$status" == "completed" ]] && { echo; break; }
        sleep 20
      done
      print_run "$rid"
    fi
    ;;
  *)
    branch="${1:-$(current_branch)}"
    if has_gh; then
      prepare_gh
      rid=$(gh run list --repo "$REPO_SLUG" --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId')
      [[ -n "$rid" ]] || die "分支 $branch 上没有任何 workflow run (还没推送, 或仓库地址不对: $REPO_SLUG)"
      run_sha=$(gh run view "$rid" --repo "$REPO_SLUG" --json headSha --jq '.headSha')
      gh_show "$rid"
      assert_verdict_matches_tip "$branch" "$run_sha" || exit $?
    else
      rid=$(latest_run_id "$branch")
      [[ -n "$rid" ]] || die "分支 $branch 上没有任何 workflow run (还没推送, 或仓库地址不对: $REPO_SLUG)"
      run_sha=$(api "$API/actions/runs/$rid" | python3 -c 'import json,sys; print(json.load(sys.stdin)["head_sha"])')
      print_run "$rid"
      assert_verdict_matches_tip "$branch" "$run_sha" || exit $?
    fi
    ;;
esac
