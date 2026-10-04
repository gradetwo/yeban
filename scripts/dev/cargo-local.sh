#!/usr/bin/env bash
# 本机 (M2 macOS) 轻量 cargo 包装器。
#
# 为什么需要它:
#   1. rust-toolchain.toml 把 L1 确定性所需的工具链钉在 `1.99.0` (ARCH-DET-001)。
#      本机 rustup 只装了名为 `stable` 的同版本工具链; 在受限沙箱里 rustup 无法写
#      ~/.rustup 去安装一个别名, 因此这里用 RUSTUP_TOOLCHAIN=stable 复用已装工具链。
#      直接在自己的终端里跑 `cargo` 时, rustup 会自动装好 `1.99.0`, 无需本脚本。
#   2. 受限沙箱不允许写 ~/.cargo; 因此把 CARGO_HOME 指向工作区内的缓存目录。
#      这只是本机开发环境的适配, CI 上用官方缓存, 语义完全一致。
#
# 纪律: 本脚本**只**用于轻量操作 (fmt / 单个无重依赖 crate 的 check+test)。
# 禁止用它跑 `--workspace` 全量构建、benchmark、fuzz, 或任何会编译 slint/cpal/
# symphonia 的命令 —— 那些一律交给 GitHub Actions (docs/DEV_WORKFLOW.md)。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export CARGO_HOME="${YEBAN_LOCAL_CARGO_HOME:-/Users/crow/work/music/.cargo-home}"
export RUSTUP_TOOLCHAIN="${YEBAN_LOCAL_RUSTUP_TOOLCHAIN:-stable}"

if [[ ! -d "$CARGO_HOME" ]]; then
  mkdir -p "$CARGO_HOME"
  echo "[cargo-local] created local CARGO_HOME at $CARGO_HOME" >&2
fi

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

cd "$repo_root"
exec cargo "$@"
