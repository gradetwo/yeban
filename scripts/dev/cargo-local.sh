#!/usr/bin/env bash
# 本机 (M2 macOS) 轻量 cargo 包装器。
#
# 它做两件事:
#   1. 环境自适应 (scripts/dev/local-env.sh): 在受限沙箱里自动把 CARGO_HOME/RUSTUP_TOOLCHAIN
#      指到可用位置; 在普通终端里**什么都不改**, 与用户自己的 rustup 配置完全一致。
#   2. 重型操作护栏: 直接拒绝 `--workspace` / `--all`。
#
# 纪律 (用户硬性要求: 本机不跑高耗 CPU 任务): 本脚本**只**用于轻量操作
# (fmt / 单个无重依赖 crate 的 check+test)。全量构建、benchmark、fuzz, 以及任何会编译
# slint / cpal / symphonia 的命令, 一律交给 GitHub Actions (docs/DEV_WORKFLOW.md)。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=./local-env.sh
source "$repo_root/scripts/dev/local-env.sh"

if [[ $# -eq 0 ]]; then
  echo "usage: cargo-local.sh <cargo args...>   (e.g. test -p yeban-model)" >&2
  exit 2
fi

# 全量/重型操作护栏: 宁可在此处失败, 也不要在 M2 上把 CPU 打满。
# 例外: `cargo fmt --all` 只做文本格式化, 不编译, 允许。
if [[ "${1:-}" != "fmt" ]]; then
  for arg in "$@"; do
    case "$arg" in
      --workspace|--all)
        echo "[cargo-local] REFUSED: '$arg' 是全量构建, 请交给 CI (docs/DEV_WORKFLOW.md)" >&2
        exit 3
        ;;
    esac
  done
fi

# ⚠ 必须 cd 到**当前目录所属的工作区**, 而不是**脚本所在**的 checkout。
# 实测事故: 本脚本住在主仓 (`/…/yeban/scripts/dev/`), 而工作线在 `.worktrees/<name>/`;
# 在 worktree 里用**主仓的绝对路径**调用它时, 旧实现 `cd "$repo_root"` 会切回主仓 ⇒
# **编译并测试的是主仓的代码**, 而报告里写的是"本工作树本机真跑" ——
# 也就是说: 那个"全绿"根本不是被测对象的绿。修复: 取**当前目录**的 git 顶层。
if workspace_root="$(git rev-parse --show-toplevel 2>/dev/null)" && [ -n "$workspace_root" ]; then
  cd "$workspace_root"
fi
printf '\033[2m[cargo-local] 工作区=%s\033[0m\n' "$(pwd)"
exec cargo "$@"
