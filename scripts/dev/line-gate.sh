#!/usr/bin/env bash
# **一条线的快速自检**（推之前跑）：只编/测"这一条线真的可能弄坏的东西"。
#
# ⚠ **这不是判决。** 本仓的判决只有 CI（AGENTS §5.5：只有 CI 的判决算数）。
# 这个脚本的职责是把"明显要红的"挡在推送之前，把**全量**留给 CI —— 于是每条线的
# 本地串行时间从"5 个 crate 全量测试 + 合并特性腿"降到"改动的 crate ＋ 它们的反向
# 依赖"。这是 2026-10-25 负责人要求「加快功能开发进度、解耦异步多线推进」之后落的
# 机械支撑，详见台账 R1137。
#
# ## 为什么是"改动的 crate ∪ 反向依赖"
#
# 只测改动的 crate 是不够的：`yeban-model` 改一个变体会让 `yeban-mcp` **编译不过**
# （2026-10-25 真实发生过：`E0599 no variant named PitchBendFirstTickNotZero`）。
# 反向依赖（谁依赖我）用 `cargo metadata` 算，**不编译**、只解析清单。
#
# ## 为什么开头要 `touch`
#
# 共享 target 里 **4759 个 `.d` 依赖文件中有 671 个指向 `.worktrees/…`**（2026-10-25
# 实测）—— 那是**别的**工作树的源码路径。cargo 判断"要不要重编"看的是 `.d` 里记下的
# 那些**绝对路径**的 mtime，于是：**本工作树改了文件，而 `.d` 记的是另一棵树的同名
# 文件 ⇒ cargo 认为没变 ⇒ 用别人的产物**。这不是理论，它是今夜 `E0599` / `E0004` /
# 「`Finished` 之后测试二进制被删」三种症状的机制解释之一。
#
# ⇒ 本脚本在编译前把**改动 crate 的 `src/**` 与 `tests/**` 全部 touch 一遍**：只多编
# 这几个 crate，却让"别人的产物冒充我的"在这一条线上**不可能发生**。
#
# ## 用法
#
#     scripts/dev/line-gate.sh                 # 对 origin/main 求差
#     scripts/dev/line-gate.sh --base <ref>
#     scripts/dev/line-gate.sh --list          # 只打印算出来的 crate 集合，不编译
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

# 本仓的执行环境（AGENTS §5.2）：共享 CARGO_HOME / 钉死工具链 / **共享** CARGO_TARGET_DIR。
# ⚠ 这里给出**默认值**，因为脚本被 `cargo metadata` 这类子进程依赖的环境一旦缺失，
# 报出来的错（exit 101）看起来像清单坏了，而实际只是环境没带。已设置的变量一律不覆盖。
ROOT=$(cd "$(git rev-parse --show-toplevel)" && pwd)
for v in CARGO_HOME XDG_CACHE_HOME RUSTUP_TOOLCHAIN CARGO_TARGET_DIR; do
  if [ -z "${!v:-}" ]; then
    case "$v" in
      CARGO_HOME)       export CARGO_HOME="$ROOT/../.cargo-home" ;;
      XDG_CACHE_HOME)   export XDG_CACHE_HOME="$ROOT/../.cache" ;;
      RUSTUP_TOOLCHAIN) export RUSTUP_TOOLCHAIN=stable ;;
      CARGO_TARGET_DIR) export CARGO_TARGET_DIR="$ROOT/target" ;;
    esac
    echo "[env] $v 未设置 ⇒ 默认用 ${!v}"
  fi
done

BASE=""
LIST_ONLY=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --base) BASE="${2:?--base 需要一个 ref}"; shift 2 ;;
    --list) LIST_ONLY=1; shift ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "未知实参: $1" >&2; exit 2 ;;
  esac
done
if [ -z "$BASE" ]; then
  BASE=$(git merge-base HEAD origin/main 2>/dev/null || git merge-base HEAD main 2>/dev/null || true)
fi
if [ -z "$BASE" ]; then
  echo "⛔ 算不出基线 ref（试过 origin/main / main）" >&2
  exit 3
fi

echo "== 基线: $BASE .. $(git rev-parse --short HEAD)"

# ---- 1. 改动的 crate -------------------------------------------------------
# ⚠ 必须同时看**已提交**、**未提交**与**未跟踪**：只比 `BASE...HEAD` 会让"还没提交的改动"
# 完全看不见 ⇒ 集合为空 ⇒ 门禁**假绿**（本脚本第一版就是这个错，2026-10-25 当场修）。
CHANGED=$( { git diff --name-only "$BASE" -- . ; git ls-files --others --exclude-standard ; } | sed -n 's#^crates/\([^/]*\)/.*#\1#p' | sort -u )
if [ -z "$CHANGED" ]; then
  echo "== 没有任何 crate 改动（只动了非 crates/** 的文件）⇒ 只跑守卫与轻门"
fi

# ---- 2. 加上反向依赖（只解析清单，不编译）---------------------------------
REVERSED=$(python3 - "$CHANGED" <<'PY'
import json, subprocess, sys
changed = [c for c in sys.argv[1].split() if c]
if not changed:
    print("")
    raise SystemExit(0)
proc = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"],
                      capture_output=True, text=True)
if proc.returncode != 0:
    sys.stderr.write(proc.stderr[-1500:])
    raise SystemExit(0)  # 让 shell 侧走"算不出反向依赖"的分支并告警
meta = json.loads(proc.stdout)
nodes = {p["id"]: p for p in meta["packages"]}
deps = {p["name"]: set() for p in meta["packages"] if p["name"].startswith("yeban")}
name_of = {p["id"]: p["name"] for p in meta["packages"]}
for n in meta["resolve"]["nodes"]:
    me = name_of.get(n["id"], "")
    for d in n["deps"]:
        dn = name_of.get(d["pkg"], "")
        if me in deps and dn in deps:
            deps[me].add(dn)
# 传递闭包：谁（直接或间接）依赖改动的 crate
extra, frontier = set(), set(changed)
while frontier:
    nxt = set()
    for name, ds in deps.items():
        if name in extra or name in frontier:
            continue
        if ds & frontier:
            nxt.add(name)
    extra |= nxt
    frontier = nxt
print(" ".join(sorted(extra)))
PY
)
if [ -z "$REVERSED" ] && [ -n "$CHANGED" ]; then
  echo "⚠ 没算出反向依赖（cargo metadata 可能失败）—— 只测改动的 crate，请自己确认这够"
fi

CRATES=$( { [ -n "$CHANGED" ] && echo "$CHANGED"; [ -n "$REVERSED" ] && echo "$REVERSED" | tr ' ' '\n'; } | sed '/^$/d' | sort -u )
if [ -z "$CRATES" ]; then
  echo "== 待编/待测集合为空"
else
  echo "== 待编/待测 crate（$(echo "$CRATES" | wc -l | tr -d ' ') 个）:"
  echo "$CRATES" | sed 's/^/   /'
fi
if [ "$LIST_ONLY" = "1" ]; then
  exit 0
fi

# ---- 3. touch：让"别人的产物"不可能冒充本树的（见文件头）------------------
if [ -n "$CHANGED" ]; then
  touched=0
  for c in $CHANGED; do
    while IFS= read -r f; do
      touch "$f" && touched=$((touched + 1))
    done < <(find "crates/$c/src" "crates/$c/tests" -name '*.rs' -type f 2>/dev/null)
  done
  echo "== 已 touch $touched 个源文件（改动 crate 的 src/** 与 tests/**）"
fi

# ---- 4. 在共享 target 的单写者锁里跑 --------------------------------------
CRATE_ARGS=()
if [ -n "$CRATES" ]; then
  # shellcheck disable=SC2207
  CRATE_ARGS=($(echo "$CRATES" | sed 's/^/-p /' | tr '\n' ' '))
fi

run() {
  local name="$1"; shift
  "$@" >"/tmp/line-gate.$$.out" 2>&1
  local rc=$?
  if [ "$rc" = "0" ]; then
    echo "[$name] ok"
  else
    echo "[$name] ⛔ rc=$rc"
    tail -20 "/tmp/line-gate.$$.out" | sed 's/^/    /'
  fi
  return "$rc"
}

fail=0
bash scripts/dev/target-lock.sh -- bash -c '
  set -uo pipefail
  fail=0
  if [ "'"${#CRATE_ARGS[@]}"'" -gt 0 ]; then
    cargo fmt '"${CRATE_ARGS[*]}"' -- --check || fail=1
    cargo clippy '"${CRATE_ARGS[*]}"' --lib --tests --locked -- -D warnings || fail=1
    cargo test '"${CRATE_ARGS[*]}"' --locked --no-fail-fast || fail=1
  else
    echo "(无 crate 改动 ⇒ 跳过 fmt/clippy/test)"
  fi
  exit $fail
'
lock_rc=$?
[ "$lock_rc" = "0" ] || fail=1

# 守卫与轻门不碰 target，放在锁外跑（可以与别的线的编译并行）
run guards python3 scripts/guards/policy_check.py || fail=1
run light bash scripts/gates/run-gates.sh light || fail=1

echo
if [ "$fail" = "0" ]; then
  echo "== 快速自检通过。⚠ 这不是判决：全量（含 app 合并特性腿与跨 crate 腿）由 CI 判。"
else
  echo "== ⛔ 快速自检未通过（见上面每条的 rc）"
fi
exit "$fail"
