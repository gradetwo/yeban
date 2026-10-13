#!/usr/bin/env bash
# 读回**某个提交自己**的 CI 判决 —— 一个命令，输出固定的七行摘要。
#
# 为什么要有它（自造工具，2026-10-13）：
#   我在 5 轮里手写了 5 次"轮询两条 run + 解析 jobs + 拼表"的脚本。
#   那段脚本每次都要重写、且有三个易错点：
#     ① 轮询没有上限（可能永不退出）；
#     ② 忘了区分"run 已结束"与"某条腿 skipped"（skipped 会显示成 success 的兄弟）；
#     ③ `gh` 的缓存目录在沙箱里不可写 ⇒ 必须把 XDG_CACHE_HOME 指到工作区内。
#   本工具把这三件事钉死，并且**先打印结论行**，让人一眼看到"能不能当绿用"。
#
# 用法:
#   scripts/dev/verdict-by-sha.sh <sha> [超时秒数，默认 3600]
#
# 退出码:
#   0 = 该提交的 run 是 success
#   1 = run 是 failure / cancelled / timed_out
#   2 = 读不到（没有该 sha 的 run，或超时）
#
# ⚠ 纪律（本仓口径）：**skipped 不是绿**。本工具会把 skipped 的腿逐条列出，
#   并在末尾打印"本次有没有 rust 腿真的执行"这一行 —— 那才是判断空绿的读数。
set -uo pipefail

SHA="${1:-}"
TIMEOUT="${2:-3600}"
if [[ -z "$SHA" ]]; then
  echo "用法: $0 <sha> [超时秒数]" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT" || exit 2
# `gh` 在受限沙箱里写不了 ~/.cache/gh ⇒ 指到工作区内的缓存（脚本自己处理，不污染全局配置）。
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$REPO_ROOT/.cache}"

# 找该 sha 的 run（可能同时有自动档与手动档）。
RUNS="$(gh run list --limit 60 --json databaseId,headSha,status,conclusion,workflowName 2>/dev/null \
  | python3 -c "
import json,sys
sha=sys.argv[1]
try:
    rows=json.load(sys.stdin)
except Exception:
    sys.exit(0)
for r in rows:
    if (r.get('headSha') or '').startswith(sha):
        print(r['databaseId'], r.get('workflowName',''), r.get('status',''), r.get('conclusion') or '-')
" "$SHA")"

if [[ -z "$RUNS" ]]; then
  echo "SHA=${SHA:0:12} 结论=NO_RUN（找不到该提交的 run）"
  exit 2
fi

deadline=$(( $(date +%s) + TIMEOUT ))
while :; do
  pending="$(printf '%s\n' "$RUNS" | awk '$3!="completed"' | wc -l | tr -d ' ')"
  [[ "$pending" == "0" ]] && break
  if (( $(date +%s) >= deadline )); then
    echo "SHA=${SHA:0:12} 结论=TIMEOUT（${TIMEOUT}s 内仍有 run 未结束）"
    exit 2
  fi
  sleep 20
  RUNS="$(gh run list --limit 60 --json databaseId,headSha,status,conclusion,workflowName 2>/dev/null \
    | python3 -c "
import json,sys
sha=sys.argv[1]
try:
    rows=json.load(sys.stdin)
except Exception:
    sys.exit(0)
for r in rows:
    if (r.get('headSha') or '').startswith(sha):
        print(r['databaseId'], r.get('workflowName',''), r.get('status',''), r.get('conclusion') or '-')
" "$SHA")"
done

overall=0
printf '%s\n' "$RUNS" | while read -r id name status conclusion; do
  [[ -z "$id" ]] && continue
  echo "RUN=$id WORKFLOW=$name 结论=$conclusion"
  gh run view "$id" --json jobs 2>/dev/null | python3 -c "
import json,sys,datetime
d=json.load(sys.stdin)
def dur(a,b):
    if not a or not b: return None
    try:
        return int((datetime.datetime.fromisoformat(b.replace('Z','+00:00'))
                   -datetime.datetime.fromisoformat(a.replace('Z','+00:00'))).total_seconds())
    except Exception:
        return None
real=0; skipped=[]
for j in d.get('jobs',[]):
    c=j.get('conclusion') or '-'
    steps=len(j.get('steps',[]))
    print(f\"  腿={j['name']!r} 结论={c} 步数={steps} 时长={dur(j.get('startedAt'),j.get('completedAt'))}s\")
    # 只有 conclusion == 'skipped' 才算跳过。
    # 不要用 steps==0 当跳过的信号：GitHub API 有时对真跑过的 job 也返回空 steps
    # 数组（实测 run 38471600290：8 条腿全部 steps=0，而 yeban-engine 与 yeban-app
    # 的结论是 success，那是真跑过）。用 steps==0 判定会把真绿误报成全跳过（空绿），
    # 反过来也会掩盖真的跳过。
    # 注意：本文件的 Python 内嵌在**双引号**里，注释里**不许出现反引号**（会被 shell
    # 当命令替换执行 —— 这个 bug 我犯过一次）。
    if c=='skipped':
        skipped.append(j['name'])
    else:
        real+=1
print(f\"  实跑腿数={real} 跳过腿数={len(skipped)}\")
print('  判定=' + ('有空绿风险（有腿被跳过）' if skipped else '全部腿都跑了'))
"
done

# 任一 run 非 success ⇒ 整体非绿。
bad="$(printf '%s\n' "$RUNS" | awk '$4!="success"' | wc -l | tr -d ' ')"
if [[ "$bad" != "0" ]]; then
  echo "SHA=${SHA:0:12} 结论=NOT_GREEN（$bad 个 run 不是 success）"
  exit 1
fi
echo "SHA=${SHA:0:12} 结论=SUCCESS"
exit 0
