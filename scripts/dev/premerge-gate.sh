#!/usr/bin/env bash
# 合并前复合门（集成者用）：一条命令跑完"会红的那几层"。
#
# 为什么有它：实测一次合并核验要**分开跑 5-6 条命令**（fmt / check / clippy /
# test / light），而且每条都要**单独取退出码**（⛔ 不能走管道 —— 管道会把退出码
# 换成最后一环的）。分开跑既慢又容易漏掉一层。
#
# ⚠ 每层的退出码**单独采集**、⛔ **不通过管道**。
# ⚠ clippy 一律跑 `--lib --tests`：那是 CI 会红的那一层（台账第 43 条）。
#    ⛔ 本机**不**跑 `--all-targets` 的 clippy（yeban-engine/yeban-app 的重依赖）。
#
# 用法：
#   scripts/dev/premerge-gate.sh yeban-model            # 单 crate
#   scripts/dev/premerge-gate.sh yeban-model yeban-mcp  # 多 crate
#   SKIP_TEST=1 scripts/dev/premerge-gate.sh yeban-app  # 跳过 cargo test（本机太慢时）
#
# 退出码：0 = 全过；1 = 至少一层红（逐层读数见 stdout 与日志）。
set -u

: "${CARGO_HOME:=/Users/crow/work/music/.cargo-home}"
: "${RUSTUP_TOOLCHAIN:=stable}"
: "${XDG_CACHE_HOME:=/Users/crow/work/music/.cache}"
: "${RUST_MIN_STACK:=16777216}"
export CARGO_HOME RUSTUP_TOOLCHAIN XDG_CACHE_HOME RUST_MIN_STACK

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT" || exit 2

[ "$#" -ge 1 ] || { echo "用法: $0 <crate> [crate...]" >&2; exit 2; }

LOGDIR="${TMPDIR:-/tmp}/premerge-gate.$$"
mkdir -p "$LOGDIR" || exit 2

FAILED=""
note() { printf '%-46s %s\n' "$1" "$2"; }

# 一层 = 一条命令；退出码**单独**采集。
layer() {
  local name="$1"; shift
  local log="$LOGDIR/$(echo "$name" | tr -c 'A-Za-z0-9' '_').log"
  "$@" >"$log" 2>&1
  local rc=$?
  if [ "$rc" -eq 0 ]; then
    note "$name" "0"
  else
    note "$name" "$rc  ⛔"
    FAILED="$FAILED|$name"
    echo "----- $name (尾 25 行) -----"
    tail -25 "$log"
    echo "-----------------------------"
  fi
  return 0
}

echo "== 合并前复合门 =="
echo "crate: $*"
echo

for c in "$@"; do
  echo "-- $c --"
  layer "fmt -p $c --check" cargo fmt -p "$c" -- --check
  layer "check -p $c --lib" cargo check -p "$c" --lib --locked
  # ⚠ 第 43 条：这一层是 CI 真正会红的地方。
  layer "clippy -p $c --lib --tests" cargo clippy -p "$c" --lib --tests --locked -- -D warnings
  # ⚠ 第 255 条：CI 有两条本机旧门**没覆盖**的腿 —— `--all-targets`（编译测试目标）与
  #    `--features in-process-mcp`。实测代价：`#86` 改了 `tests/production_save_ui.rs` 的
  #    一处 `Rc` 用法，**默认腿绿**而**特性腿 `error[E0433]`** ⇒ 主干红了一个 run。
  #    下面两条就是补那两条腿；⛔ 只在 crate 真声明了该特性时才跑特性腿。
  layer "check -p $c --all-targets" cargo check -p "$c" --all-targets --locked
  if grep -q '^in-process-mcp' "crates/$c/Cargo.toml" 2>/dev/null; then
    layer "check -p $c --all-targets --features in-process-mcp" \
      cargo check -p "$c" --all-targets --locked --features in-process-mcp
  fi
  if [ "${SKIP_TEST:-0}" = "1" ]; then
    note "test -p $c (SKIP_TEST=1 ⇒ 没跑)" "跳过"
  else
    layer "test -p $c --lib" cargo test -q -p "$c" --lib --locked
  fi
  echo
done

echo "-- 仓级轻门 --"
layer "run-gates.sh light" bash scripts/gates/run-gates.sh light

echo
if [ -z "$FAILED" ]; then
  echo "合并前复合门: 全过 ✓"
  exit 0
fi
echo "合并前复合门: 红层 =${FAILED}"
exit 1
