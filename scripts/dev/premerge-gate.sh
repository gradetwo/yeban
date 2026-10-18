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
#   scripts/dev/premerge-gate.sh --strict yeban-render  # ⭐ 下游未跑就失败（`#199`）
#   SKIP_TEST=1 scripts/dev/premerge-gate.sh yeban-app  # 跳过 cargo test（本机太慢时）
#
# ⭐ `#199`（落实 `#197` 的建议②）：**受影响 crate 集合进门**。
#   只传一个 crate 时，本门用 `cargo metadata --no-deps`（0.15 秒，只读成员清单）推出
#   「谁依赖它」的**传递下游**；下游里有成员 crate 没被传入 ⇒ 打一条**显式警告**列出
#   还该跑的 crate。
#   ⚠ 为什么必须警告而不是沉默：`#162` / `#169` / `#197` 三次红的形状**完全一样** ——
#     改了共享的常量/容量/样例/谓词，**只跑了改动的那一个 crate**，而别的 crate 里
#     读它的判据红了。CI 的 `plan` 会算这个集合（`scripts/dev/changed-crates.py`），
#     本机旧门只会"你传什么就测什么"。
#   ⚠ 为什么**不**强制失败：既有用法（`... yeban-model`）一直是只测一个 crate，
#     突然变 rc=1 会把一条"提醒"变成一次"破坏"。默认只警告，`--strict` 档才失败。
#
# 退出码：0 = 全过；1 = 至少一层红（逐层读数见 stdout 与日志）；
#         2 = 用法错误（没有 crate / 未知选项）。
set -u

: "${CARGO_HOME:=/Users/crow/work/music/.cargo-home}"
: "${RUSTUP_TOOLCHAIN:=stable}"
: "${XDG_CACHE_HOME:=/Users/crow/work/music/.cache}"
: "${RUST_MIN_STACK:=16777216}"
export CARGO_HOME RUSTUP_TOOLCHAIN XDG_CACHE_HOME RUST_MIN_STACK

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT" || exit 2

# ⭐ `#199`：把 `--strict` 这类型开关**先摘出去**，否则它会被当成 crate 名传给 cargo。
# ⛔ 不要动既有用法：`$1` 仍是第一个 crate，不带任何开关时行为与旧版逐字一致。
STRICT=0
CRATES=()
for arg in "$@"; do
  case "$arg" in
    --strict) STRICT=1 ;;
    -*) echo "未知选项: $arg（用法: $0 [--strict] <crate> [crate...]）" >&2; exit 2 ;;
    *) CRATES+=("$arg") ;;
  esac
done
[ "${#CRATES[@]}" -ge 1 ] || { echo "用法: $0 [--strict] <crate> [crate...]" >&2; exit 2; }

LOGDIR="${TMPDIR:-/tmp}/premerge-gate.$$"
mkdir -p "$LOGDIR" || exit 2

FAILED=""
note() { printf '%-46s %s\n' "$1" "$2"; }

# ⭐ `#199`（落实 `#197` 的建议②）：由 `cargo metadata --no-deps` 推出两个集合。
#   ⚠ 成本实测 0.145 秒（`cargo metadata --no-deps` 只读成员清单，⛔ 不解析第三方依赖、
#     ⛔ 不往仓库里落任何文件）。输出三行 `键: 值`：
#     `down:` 传入 crate 的**直接**下游（成员清单里真写了它的那些）——「还该跑」的主集合
#     `deep:` 更远的**传递**下游（经由上面那些；与 CI 的 plan 同一口径）
#     `up:`   传入 crate 的**传递上游**（仅供人读，⛔ 不参与 --strict）
#   ⚠ 与 CI 的 `scripts/dev/changed-crates.py` 同源：那边也是"看谁依赖谁"，
#     只是它从 `git diff` 出发、本门从「你传了哪些 crate」出发。
#   ⚠ metadata 失败（锁不同步 / 不在工作区里）⇒ 打一行 `skip`，⛔ 不失败：
#     这条机制是**提醒**，不该让一条本来能跑的门因为推导失败而变红。
affected_sets() {
  # ⚠ 必须 **export**，⛔ 不能写成 `PASSED=… cargo … | python3` 的前缀赋值：
  #    前缀赋值只作用于管道的**左边那一环**，右边的 python 看不见它 ⇒ 推出空集合、
  #    于是**静默地**不警告（本机制的第一版正是这样，实测四组输入全部"无下游"）。
  # ⚠ 数组必须写成 `${CRATES[@]+"${CRATES[@]}"}`：本机 macOS 自带 **bash 3.2**，
  #    在 `set -u` 下展开空数组会报 `unbound variable` —— ⚠ `bash -n` **抓不到**
  #    （那是运行时语义），是 `policy_check.py` 的 G14 抓到的。
  PASSED="$(printf '%s ' ${CRATES[@]+"${CRATES[@]}"})"
  export PASSED
  cargo metadata --no-deps --locked --format-version 1 2>/dev/null |
    python3 -c '
import json, os, sys

passed = os.environ.get("PASSED", "").split()
try:
    meta = json.load(sys.stdin)
except Exception:
    print("skip: cargo metadata 没给出可解析的 JSON")
    raise SystemExit(0)

names = {p["name"] for p in meta["packages"]}
edges = {
    p["name"]: {d["name"] for d in p["dependencies"] if d["name"] in names}
    for p in meta["packages"]
}
dependents = {n: {o for o, ds in edges.items() if n in ds} for n in names}


def closure(start, step):
    seen, frontier = set(), {start}
    while frontier:
        nxt = set()
        for c in frontier:
            nxt |= step(c) - seen
        seen |= nxt
        frontier = nxt
    seen.discard(start)
    return seen


known = {c for c in passed if c in names}
direct, deep, up = set(), set(), set()
for c in known:
    # 直接下游 = 成员清单里真写了 `c` 的那些；传递下游 = 再往上闭包一层。
    direct |= {o for o, ds in edges.items() if c in ds}
    deep |= closure(c, lambda x: dependents.get(x, set()))
    up |= closure(c, lambda x: edges.get(x, set()))
direct -= known
deep = deep - known - direct
print("down: " + " ".join(sorted(direct)))
print("deep: " + " ".join(sorted(deep)))
print("up: " + " ".join(sorted(up - known - direct - deep)))
unknown = sorted(set(passed) - names)
if unknown:
    print("unknown: " + " ".join(unknown))
'
}

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
echo "crate: ${CRATES[*]}"
[ "$STRICT" = "1" ] && echo "档位: --strict（下游未跑 ⇒ 失败）"
echo

# ---- ⭐ 受影响 crate 集合（`#199` 建议②）：先于各层打印，**一定**出现在输出里 ----
SETS="$(affected_sets)"
DOWN="$(printf '%s\n' "$SETS" | sed -n 's/^down: *//p' | head -1)"
DEEP="$(printf '%s\n' "$SETS" | sed -n 's/^deep: *//p' | head -1)"
UP="$(printf '%s\n' "$SETS" | sed -n 's/^up: *//p' | head -1)"
UNKNOWN="$(printf '%s\n' "$SETS" | sed -n 's/^unknown: *//p' | head -1)"

echo "-- 受影响 crate 集合（⭐ #199 建议②，数据源 = cargo metadata --no-deps）--"
if [ -z "$SETS" ]; then
  echo "⚠ 推导没跑起来（cargo metadata 无输出）⇒ 手工确认下游; 本行⛔不参与失败判定"
elif printf '%s\n' "$SETS" | grep -q '^skip'; then
  echo "⚠ 推导没跑起来（cargo metadata 失败）⇒ 手工确认下游; 本行⛔不参与失败判定"
else
  [ -n "$UNKNOWN" ] && echo "⚠ 不在工作区成员里（跳过推导）: ${UNKNOWN}"
  if [ -n "$DOWN" ] || [ -n "$DEEP" ]; then
    [ -n "$DOWN" ] && echo "⚠ 下游未跑（直接依赖传入的 crate）: ${DOWN}"
    [ -n "$DEEP" ] && echo "⚠ 下游未跑（传递: 经由上面那些）: ${DEEP}"
    echo "   ⇒ 它们的判据会读传入 crate 的公共面/常量/样例表。"
    echo "   ⇒ 这正是 \`#162\` / \`#169\` / \`#197\` 三次红的形状：改了共享的东西，"
    echo "     只跑了改动的那一个 crate，别的 crate 里读它的判据红了。"
    echo "   ⇒ 还该跑: bash scripts/dev/premerge-gate.sh ${CRATES[*]} ${DOWN} ${DEEP}"
    if [ "$STRICT" = "1" ]; then
      echo "   ⇒ --strict: 本条 ⇒ 失败（rc=1）"
    else
      echo "   ⇒ 默认档不失败（⛔ 会破坏既有用法）；加 --strict 才失败。"
    fi
  else
    echo "✓ 无下游成员 crate 依赖 ${CRATES[*]}（叶子 crate 或下游已在集合里）"
  fi
  [ -n "$UP" ] && echo "ℹ 上游未跑: ${UP}（传入的 crate 依赖它们；只改自己的消费面时可忽略，⛔ 不参与 --strict）"
fi
echo

for c in ${CRATES[@]+"${CRATES[@]}"}; do
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
    # ⭐ 第 303 条的同款：这一层原来也是 **check** ⇒ `midi-input` 下的判据本机从未执行。
    layer "test -p $c --features midi-input" \
      cargo test -p "$c" --locked --no-fail-fast --features midi-input
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
# ⭐ `#199`：`--strict` 的唯一作用点 —— 下游没跑就记一条红层（各层**照常全部跑完**，
# ⛔ 不提前退出：这样一次调用仍能给出全部读数）。
if [ "$STRICT" = "1" ] && [ -n "${DOWN}${DEEP}" ]; then
  FAILED="$FAILED|affected-set(下游未跑: ${DOWN} ${DEEP})"
fi
if [ -z "$FAILED" ]; then
  echo "合并前复合门: 全过 ✓"
  exit 0
fi
echo "合并前复合门: 红层 =${FAILED}"
exit 1
