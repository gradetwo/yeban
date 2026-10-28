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

# ---- 重依赖 crate 的**纯模块**回落（本机也能 lint 的那一部分）--------------
# 为什么要有这一段：`[skip] …交给 CI` 对**整个 crate** 成立，但 `yeban-mcp` 里最常被改的
# `src/domain/section_build.rs` **只依赖** `yeban-model` / `yeban-theory` / `serde_json`，
# 仓库里已有零重依赖脚手架 `crates/yeban-mcp/verify/section_pure.rs` —— 它用 `#[path]` 引的
# 就是那份真实源文件。把脚手架文档里的 `rustc` 换成 **`clippy-driver`**（rustc 的包装器）
# 并加 `--emit=metadata`（不链接，实测 1.7s），本机就能 lint 到它。
# 实测依据（2026-10-28）：对 `d654a854` 零诊断；指向其父 `5a176d4c` 的同一文件 ⇒ 逐字复现
# run `40609755301` 的红 `this function has too many arguments (8/7)`。那一条在 CI 上的代价
# 是**一整轮 ≈45 分钟**（三条腿同红、0 条 `test result:`）。
# ⚠ 覆盖边界：脚手架引不到的 `crates/yeban-mcp/tests/**` 要整个 lib（拖 `yeban-render`）
# ⇒ 那一类仍**只能交给 CI**，本脚本⛔ 声称覆盖该 crate 的全部判据。
pure_scaffold_for() {  # $1=crate；置 PURE_SCAFFOLD / PURE_DEPS，非纯模块 crate 返回 1
  PURE_SCAFFOLD=""; PURE_DEPS=""
  case "$1" in
    yeban-mcp)
      PURE_SCAFFOLD="crates/yeban-mcp/verify/section_pure.rs"
      PURE_DEPS="-p yeban-model -p yeban-theory"
      ;;
    *) return 1 ;;
  esac
  [[ -f "$PURE_SCAFFOLD" ]]
}

run_pure_clippy() {  # $1=crate
  local c="$1" deps_dir f name
  command -v clippy-driver >/dev/null 2>&1 || {
    echo "[skip] $c 有纯模块脚手架但本机无 clippy-driver ⇒ 交给 CI"; return 0; }
  deps_dir="${CARGO_TARGET_DIR:-$REPO/target}/debug/deps"
  if ! bash scripts/dev/cargo-local.sh build $PURE_DEPS >/tmp/clippy-pure-build.log 2>&1; then
    echo "[skip] $c 的纯模块预构建失败 ⇒ 交给 CI（日志 /tmp/clippy-pure-build.log）"; return 0
  fi
  local -a externs=()
  for name in yeban_model yeban_theory serde_json; do
    f=$(ls -t "$deps_dir"/lib"${name}"-*.rlib 2>/dev/null | head -1 || true)
    if [[ -z "$f" ]]; then
      echo "[skip] 找不到 ${name} 的 rlib ⇒ 纯模块 clippy 交给 CI（不是通过）"; return 0
    fi
    externs+=(--extern "${name}=${f}")
  done
  # 根清单的 `[workspace.package] edition`；取不到就按 2024（当前值）
  local edition; edition=$(sed -n 's/^edition = "\{0,1\}\([^"]*\)"\{0,1\}.*/\1/p' Cargo.toml | head -1)
  # 旗标 = 根 `Cargo.toml` 的 `[workspace.lints]`（`all`/`dbg_macro`/
  # `undocumented_unsafe_blocks` 均 deny）＋ `[lints.rust]` 两条。改那里要同步改这里。
  echo "[run ] clippy-driver（纯模块脚手架）$c → $PURE_SCAFFOLD"
  # ⚠ `--emit=metadata` 不给输出位置会把 `libsection_pure.rmeta` **落在仓库根**（实测），
  # 所以固定输出到 /tmp；目录由 rustc 自己创建。
  local out_dir="/tmp/clippy-pure.out.$c"
  if ! clippy-driver --edition "${edition:-2024}" --test --emit=metadata \
       --out-dir "$out_dir" \
       -D warnings -D clippy::all -D clippy::dbg_macro \
       -D clippy::undocumented_unsafe_blocks -D unsafe_op_in_unsafe_fn \
       -L dependency="$deps_dir" ${externs[@]+"${externs[@]}"} "$PURE_SCAFFOLD" \
       >/tmp/clippy-pure-$c.log 2>&1; then
    echo "[FAIL] $c 的纯模块 clippy 有诊断（诊断见 /tmp/clippy-pure-$c.log）:"
    grep -aE '^(error|warning)' "/tmp/clippy-pure-$c.log" | head -8 || true
    return 1
  fi
  return 0
}

for c in $crates; do
  [[ -f "crates/$c/Cargo.toml" ]] || continue
  if python3 scripts/dev/heavy-deps.py "$c" >/dev/null 2>&1; then
    if pure_scaffold_for "$c"; then
      run_pure_clippy "$c" || status=1
    else
      echo "[skip] $c 含重依赖且无纯模块脚手架 ⇒ 本机不编译, clippy 交给 CI"
    fi
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
