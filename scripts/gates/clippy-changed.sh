#!/usr/bin/env bash
# [规则变更，负责人授权 2026-10-05] 对**本次改动涉及且非重依赖**的 crate 跑 clippy -D warnings。
#
# 为什么加它（实测、可复现）: 我给 `yeban-dsp` 加判据时本机只跑了 `test` 与 `run-gates.sh light`,
# 两者都**不含 clippy**, 于是 `error: using chunks_exact with a constant chunk size` 走到 CI,
# 把 `rust (yeban-dsp)` 与 `windows` **两条腿**变红（run 37327050901 = failure）。
# 而 `AGENTS.md` DoD 第 1 条**要求** clippy 零告警, §5.1 也**允许**本机跑无重依赖 crate 的 clippy。
# ⇒ 机制缺的不是许可, 是**默认路径**。本脚本把它补进 `light`。
#
# 成本控制: 只对 git 改动涉及的 crate 跑; 含重依赖的按既有纪律跳过并注明交给 CI。
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
source scripts/dev/local-env.sh 2>/dev/null || true

base="${YEBAN_CLIPPY_BASE:-origin/main}"
if ! git rev-parse --verify -q "$base" >/dev/null; then
  echo "[skip] 找不到基线 $base ⇒ 本检查跳过（不是通过）"
  exit 0
fi

# 与**工作树**比（含未提交改动）—— 本机检查的意义就在于提交前抓住问题。
changed=$(git diff --name-only "$base" -- 'crates/*' 2>/dev/null || true)
if [[ -z "$changed" ]]; then
  echo "[ok] 本次改动未触及 crates/** ⇒ 无需 clippy"
  exit 0
fi
crates=$(printf '%s\n' "$changed" | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u)
[[ -n "$crates" ]] || { echo "[ok] 改动不含具体 crate ⇒ 无需 clippy"; exit 0; }

status=0
for c in $crates; do
  [[ -f "crates/$c/Cargo.toml" ]] || continue
  if python3 scripts/dev/heavy-deps.py "$c" >/dev/null 2>&1; then
    echo "[skip] $c 含重依赖 ⇒ 本机不编译, clippy 交给 CI"
    continue
  fi
  echo "[run ] clippy -p $c --all-targets -- -D warnings"
  if ! bash scripts/dev/cargo-local.sh clippy -p "$c" --all-targets -- -D warnings >/tmp/clippy-$c.log 2>&1; then
    echo "[FAIL] $c 的 clippy 有诊断（诊断见 /tmp/clippy-$c.log）:"
    grep -aE '^(error|warning)' /tmp/clippy-$c.log | head -5 || true
    status=1
  fi
done
[[ $status -eq 0 ]] && echo "[ok] 改动涉及的轻 crate clippy 零告警"
exit $status
