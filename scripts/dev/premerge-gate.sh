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
  # ⚠ 第 255 条：CI 有一批本机旧门**结构上覆盖不到**的腿 —— `--all-targets`（编译测试目标）
  #    与三条 `--features`。实测代价：`#86` 改了 `tests/production_save_ui.rs` 的一处 `Rc`
  #    用法，**默认腿绿**而**特性腿 `error[E0433]`** ⇒ 主干红了一个 run（第 256 条解释了
  #    为什么那个文件在默认腿下"绿得毫无信息量"：它是 crate 根 `#![cfg(feature = "...")]`，
  #    默认档下整个测试目标**编译成空 crate**）。
  #    下面这组层**逐条对齐 `.github/workflows/ci.yml` 里真实存在的 feature 腿**；
  #    ⛔ 只在 crate 真声明了该特性时才跑（对其余 crate 传不存在的 `--features` 会直接报错）。
  layer "check -p $c --all-targets" cargo check -p "$c" --all-targets --locked
  # CI: clippy/test (实验性导出特性)
  if grep -q '^experimental-logic-export' "crates/$c/Cargo.toml" 2>/dev/null; then
    layer "check -p $c --all-targets --features 实验性导出" \
      cargo check -p "$c" --all-targets --locked \
      --features experimental-als-export,experimental-logic-export
    # ⭐ `#145` 实测发现的洞：上面那一层是 **check**，而"引脚字节流"判据住在
    # `src/als.rs` / `src/logic.rs` 的 `#[cfg(test)]` 里 ⇒ check 编译得出来但**不会跑**
    # ⇒ 本机门曾经漏掉 `#142` 引起的那条红（CI 的 `ci.yml` 才是真 test）。
    # ⇒ 这一层补上**真 test**（双 feature 一起给：只给 logic 时 `als` 模块根本不编译）。
    layer "test -p $c --features 实验性导出" \
      cargo test -p "$c" --locked --no-fail-fast \
      --features experimental-als-export,experimental-logic-export
  fi
  # CI: clippy/test (midi-input 特性)
  if grep -q '^midi-input' "crates/$c/Cargo.toml" 2>/dev/null; then
    layer "check -p $c --all-targets --features midi-input" \
      cargo check -p "$c" --all-targets --locked --features midi-input
  fi
  # CI: test (in-process-mcp 特性)
  if grep -q '^in-process-mcp' "crates/$c/Cargo.toml" 2>/dev/null; then
    # ⭐ `#167`：这一层原来是 **check**（注释却写 "CI: test"）⇒ 该 feature 下
    # **63 个二进制 / 704 条判据一次都没在本机跑过**。⭐ 第 303 条同类洞的第二次出现。
    layer "test -p $c --features in-process-mcp" \
      cargo test -p "$c" --locked --no-fail-fast --features in-process-mcp
  fi
  # test (experimental-logic-import 特性) —— `#143` 的只读 Logic 探针腿。
  # ⚠ 这一条**必须是 test 而不止 check**：那 8 个 demo 的读数判据住在
  # `tests/logic_inspect.rs`，`check` 编译得出来但**不会跑**它。
  # ⭐ 判据本身**路径门控**（demo 不在 ⇒ skip、不红）⇒ 无 demo 的机器上安全。
  if grep -q '^experimental-logic-import' "crates/$c/Cargo.toml" 2>/dev/null; then
    layer "test -p $c --features experimental-logic-import" \
      cargo test -p "$c" --locked --features experimental-logic-import
  fi
  # 收口：把**声明了但上面没覆盖**的特性（如 `ui-test-port` / `ui-mcp-http`）一并编一遍。
  # 手动档 `gates-manual.yml` 用的就是 `--all-features`；⚠ 它不替代上面逐条腿
  # （互斥特性组合与单项腿可能不同），只作兜底。⛔ 只对真声明了 `[features]` 的 crate 跑。
  if grep -q '^\[features\]' "crates/$c/Cargo.toml" 2>/dev/null; then
    layer "check -p $c --all-targets --all-features" \
      cargo check -p "$c" --all-targets --locked --all-features
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
