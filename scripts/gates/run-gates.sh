#!/usr/bin/env bash
# 夜半门禁执行器 (Gate runner)。
#
# 规范来源: AGENTS.md §3 (DoD), docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §5
#          docs/DEV_WORKFLOW.md (本机轻量 / CI 全量的分工)
#
# 三种档位:
#   light          格式 + 机械红线守卫。零编译, 任何机器都能跑。
#   crate <name>   在 light 基础上, 对**指定 crate** 跑 clippy + test。
#                  若该 crate 引入了重依赖 (slint/cpal/symphonia/...), 本机档位会拒绝,
#                  必须交给 CI —— 这正是「开发与测试解耦、异步」的落点。
#   full           workspace 全量门禁 (fmt/clippy/test/deny/schema/guards)。
#                  只在 CI 上跑; 本机执行需显式 YEBAN_ALLOW_HEAVY=1 才放行。
#
# 绝不接受管道化的门禁 (SKILL 规则 4): 所有命令的退出码都被显式检查,
# 任何一步红就立刻以非零码退出, 不允许 `| tail` 吞掉失败。
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"

MODE="${1:-light}"
shift || true

HEAVY_RE='^[[:space:]]*(slint|slint-build|i-slint-[a-z-]*|cpal|symphonia|rubato|rayon|clack|nih-plug|vst3-sys|zip|flate2|hound|midly|midir|notify|rstar|signalsmith-stretch)[[:space:]]*='

step() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }
fail() { printf '\033[31mFAIL\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[32mok\033[0m   %s\n' "$*"; }

run() {
  local label="$1"; shift
  "$@" || fail "$label (exit=$?)"
  ok "$label"
}

gate_fmt() {
  step "cargo fmt --all --check"
  run "fmt" cargo fmt --all --check
}

gate_guards() {
  step "机械红线守卫 (scripts/guards/policy_check.py)"
  run "guards" python3 scripts/guards/policy_check.py
}

gate_schemas() {
  step "JSON Schema 契约校验"
  run "schemas" python3 scripts/gates/validate_schemas.py
}

gate_deny() {
  step "cargo deny check (开源合规)"
  # 优先用 PATH 里的 cargo-deny; 也可以用预编译二进制并通过 YEBAN_CARGO_DENY 指过来
  # (docs/DEV_WORKFLOW.md: 不要为了装它在本机做一次重编译)。
  local deny_bin="${YEBAN_CARGO_DENY:-$(command -v cargo-deny || true)}"
  if [[ -z "$deny_bin" ]]; then
    fail "cargo-deny 未安装 (CI 上由 EmbarkStudios/cargo-deny-action 提供; 本机见 docs/DEV_WORKFLOW.md)"
  fi
  run "cargo-deny" "$deny_bin" --all-features check
}

heavy_deps_of() {
  local crate="$1" manifest="crates/$1/Cargo.toml"
  [[ -f "$manifest" ]] || manifest="spikes/$1/Cargo.toml"
  [[ -f "$manifest" ]] || return 1
  grep -E "$HEAVY_RE" "$manifest" 2>/dev/null | grep -v '^[[:space:]]*#'
}

gate_crate() {
  local crate="$1"
  if [[ -z "${YEBAN_ALLOW_HEAVY:-}" ]] && heavy_deps_of "$crate" >/dev/null; then
    printf '\033[33mSKIP\033[0m %s 含重依赖, 本机不编译 (交给 CI; 见 docs/DEV_WORKFLOW.md)\n' "$crate"
    return 0
  fi
  step "crate $crate: clippy --all-targets -D warnings"
  run "clippy[$crate]" cargo clippy -p "$crate" --all-targets -- -D warnings
  step "crate $crate: test"
  run "test[$crate]" cargo test -p "$crate"
}

case "$MODE" in
  light)
    gate_fmt
    gate_guards
    ;;
  crate)
    [[ $# -ge 1 ]] || fail "用法: run-gates.sh crate <crate-name> [更多 crate...]"
    gate_fmt
    gate_guards
    for crate in "$@"; do gate_crate "$crate"; done
    ;;
  deny)
    gate_deny
    ;;
  full)
    if [[ -z "${CI:-}" && -z "${YEBAN_ALLOW_HEAVY:-}" ]]; then
      fail "full 档位只能跑在 CI 上。本机请用 light / crate <name>, 或显式 YEBAN_ALLOW_HEAVY=1。"
    fi
    gate_fmt
    gate_guards
    gate_schemas
    step "cargo clippy --workspace --all-targets -- -D warnings"
    run "clippy[workspace]" cargo clippy --workspace --all-targets -- -D warnings
    step "cargo test --workspace --all-targets"
    run "test[workspace]" cargo test --workspace --all-targets
    gate_deny
    ;;
  *)
    fail "未知档位 '$MODE' (可选: light | crate <name> | full)"
    ;;
esac

printf '\n\033[32m门禁通过\033[0m (mode=%s)\n' "$MODE"
