#!/usr/bin/env bash
# 读取 GitHub Actions 的判决 (verdict)。
#
# 为什么需要它: docs/skills/yeban-dev-workflow/SKILL.md「Honesty rules」——
#   「本地绿不是绿; 尚未读取的判决是 pending」。
# 本机不编译重依赖, 所以真正的判决来自 CI; 这个脚本负责**把判决读回来**。
#
# 仓库是公开的, 因此匿名 REST API 足以读取 run / job / step 状态与结论。
# 若提供了 GH_TOKEN (或 GITHUB_TOKEN), 还能直接拉取失败 job 的原始日志。
#
# 用法:
#   scripts/dev/ci-verdict.sh                  # 当前分支最新一次 run
#   scripts/dev/ci-verdict.sh website          # 指定分支
#   scripts/dev/ci-verdict.sh --logs <run-id>  # 拉取失败 job 日志 (需要 GH_TOKEN)
#   scripts/dev/ci-verdict.sh --watch [branch] # 轮询直到 run 结束
set -uo pipefail

REPO_SLUG="${YEBAN_REPO_SLUG:-gradetwo/yeban}"
API="https://api.github.com/repos/$REPO_SLUG"
AUTH=()
[[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]] && AUTH=(-H "Authorization: Bearer ${GH_TOKEN:-$GITHUB_TOKEN}")

die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
api() { curl -fsSL "${AUTH[@]}" -H "Accept: application/vnd.github+json" "$@"; }

current_branch() { git -C "$(dirname "${BASH_SOURCE[0]}")/../.." rev-parse --abbrev-ref HEAD; }

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

fetch_logs() {
  local run_id="$1"
  [[ ${#AUTH[@]} -gt 0 ]] || die "拉取原始日志需要 GH_TOKEN (公开仓库的日志下载接口要求鉴权)"
  local tmp; tmp="$(mktemp -d)"
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
  rm -rf "$tmp"
}

case "${1:-}" in
  --logs)  shift; fetch_logs "${1:?用法: ci-verdict.sh --logs <run-id>}" ;;
  --watch)
    shift
    branch="${1:-$(current_branch)}"
    while :; do
      rid=$(latest_run_id "$branch")
      [[ -n "$rid" ]] || die "分支 $branch 上没有任何 workflow run"
      status=$(api "$API/actions/runs/$rid" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
      printf '\r[watch] run=%s status=%s   ' "$rid" "$status"
      [[ "$status" == "completed" ]] && { echo; break; }
      sleep 20
    done
    print_run "$rid"
    ;;
  *)
    branch="${1:-$(current_branch)}"
    rid=$(latest_run_id "$branch")
    [[ -n "$rid" ]] || die "分支 $branch 上没有任何 workflow run (还没推送, 或仓库地址不对: $REPO_SLUG)"
    print_run "$rid"
    ;;
esac
