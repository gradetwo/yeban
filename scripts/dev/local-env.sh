#!/usr/bin/env bash
# 本机环境自适应 —— 被其它脚本 **source** 使用, 不要直接执行。
#
# 它要解决的问题 (docs/DEVELOPMENT_LEDGER.md §1 实测记录):
#   1. rust-toolchain.toml 把工具链钉在 `1.99.0` (L1 声学确定性要求)。
#      在**受限沙箱**里 rustup 无法写 ~/.rustup 去安装这个别名, 于是 `cargo` 一启动就报
#      "could not create temp file ... Operation not permitted"。
#   2. 同理, cargo 无法写 ~/.cargo 的 registry 缓存, 连下载依赖都会失败。
#   3. `gh` 也有同样问题 (~/.cache/gh)。
#
# 策略: **自动探测, 不硬改**。
#   - 在 CI 上 (CI 环境变量存在) 什么都不做 —— runner 的环境本来就是对的和可写的;
#   - 在本机, 只有当 ~/.cargo 或 ~/.rustup **不可写**时才退回到工作区内的目录,
#     并复用已安装的同版本工具链 (RUSTUP_TOOLCHAIN=stable)。
#   这样普通终端里的行为与用户的个人配置完全一致, 而受限环境会自愈。
#
# 覆盖方式 (可选): YEBAN_LOCAL_CARGO_HOME / YEBAN_LOCAL_RUSTUP_TOOLCHAIN / YEBAN_FORCE_LOCAL_ENV=1

_yeban_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
_yeban_workspace_root="$(cd "$_yeban_repo_root/.." && pwd)"

yeban_local_env() {
  if [[ -n "${CI:-}" ]]; then
    return 0
  fi

  local want_local="${YEBAN_FORCE_LOCAL_ENV:-}"
  if [[ -z "$want_local" ]]; then
    local cargo_dir="${CARGO_HOME:-$HOME/.cargo}"
    local rustup_dir="${RUSTUP_HOME:-$HOME/.rustup}"
    if [[ ! -w "$cargo_dir" || ! -w "$rustup_dir" ]]; then
      want_local=1
    fi
  fi

  if [[ -n "$want_local" ]]; then
    export CARGO_HOME="${YEBAN_LOCAL_CARGO_HOME:-$_yeban_workspace_root/.cargo-home}"
    export RUSTUP_TOOLCHAIN="${YEBAN_LOCAL_RUSTUP_TOOLCHAIN:-stable}"
    mkdir -p "$CARGO_HOME" 2>/dev/null || true
    # gh 的缓存目录同理 (scripts/dev/ci-verdict.sh 也会用到)。
    # 注意: 必须落在**仓库之外** —— gh 会把 run-log zip 写进缓存目录,
    # 指到仓库内就有被 `git add -A` 一起提交的风险。
    if [[ ! -w "${XDG_CACHE_HOME:-$HOME/.cache}" ]]; then
      export XDG_CACHE_HOME="${YEBAN_LOCAL_CACHE_HOME:-$_yeban_workspace_root/.cache}"
      mkdir -p "$XDG_CACHE_HOME" 2>/dev/null || true
    fi
  fi
}

yeban_local_env
