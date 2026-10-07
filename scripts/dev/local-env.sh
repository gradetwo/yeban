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
#   - 在本机, 只有当 ~/.cargo 或 ~/.rustup **不可写**时才退回到**工作区根 (仓库的父目录)**,
#     并复用已安装的同版本工具链 (RUSTUP_TOOLCHAIN=stable)。
#   这样普通终端里的行为与用户的个人配置完全一致, 而受限环境会自愈。
#
# 缓存目录策略 (硬性): 退回到的缓存目录**绝不落在仓库之内**。
#   原因: 缓存目录一旦落在仓库内, 一次 `git add -A` 就可能把它提交进历史 (本仓真发生过一次 gh 日志 zip)。
#   实现: 推导出来的位置先与 $_yeban_repo_root 比对, 命中仓库内就换用 $HOME/.cargo 与 /tmp。
#
# 覆盖方式 (可选): YEBAN_LOCAL_CARGO_HOME / YEBAN_LOCAL_RUSTUP_TOOLCHAIN / YEBAN_FORCE_LOCAL_ENV=1

_yeban_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
_yeban_workspace_root="$(cd "$_yeban_repo_root/.." && pwd)"

# 把一个推导出来的缓存目录落到仓库之外。
#   为什么: 目录名与 `git add -A` 的组合能让任何共享脚本制造出体积巨大的误提交。
#   判据: (a) 不在 $_yeban_repo_root 之内, (b) 现在就能写或能被 mkdir -p 建出来。
#   两档兜底: $HOME/.cargo (cargo 自己的默认值) ⇒ 最后是 /tmp/yeban-cargo-home。
yeban_local_cache_dir() {
  local candidate="$1" probe
  for candidate in "$1" "$HOME/.cargo" "/tmp/yeban-cargo-home"; do
    [[ "$candidate" == "$_yeban_repo_root" || "$candidate" == "$_yeban_repo_root"/* ]] && continue
    if [[ -w "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
    probe="$(dirname "$candidate")"
    while [[ ! -d "$probe" && "$probe" != "/" ]]; do probe="$(dirname "$probe")"; done
    if [[ -w "$probe" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

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
    # 默认值是**仓库的父目录** (本仓的工作区根口径: /Users/crow/work/music), 不是仓库自身。
    # 这里刻意写成 $_yeban_repo_root/.. 而不是 $_yeban_workspace_root: 两个变量的含义不同,
    # 而"缓存不在仓库里"这条不变式必须能从这一行直接读出来 (见 yeban_local_cache_dir)。
    local cargo_home="${YEBAN_LOCAL_CARGO_HOME:-$_yeban_repo_root/../.cargo-home}"
    if ! cargo_home="$(yeban_local_cache_dir "$cargo_home")"; then
      printf 'yeban local-env: 找不到仓库之外的可写缓存目录; 请设置 YEBAN_LOCAL_CARGO_HOME\n' >&2
      return 0
    fi
    export CARGO_HOME="$cargo_home"
    export RUSTUP_TOOLCHAIN="${YEBAN_LOCAL_RUSTUP_TOOLCHAIN:-stable}"
    mkdir -p "$CARGO_HOME" 2>/dev/null || true
    # gh 的缓存目录同理 (scripts/dev/ci-verdict.sh 也会用到)。
    # 注意: 必须落在**仓库之外** —— gh 会把 run-log zip 写进缓存目录,
    # 指到仓库内就有被 `git add -A` 一起提交的风险。
    if [[ ! -w "${XDG_CACHE_HOME:-$HOME/.cache}" ]]; then
      local cache_home
      if cache_home="$(yeban_local_cache_dir "${YEBAN_LOCAL_CACHE_HOME:-$_yeban_repo_root/../.cache}")"; then
        export XDG_CACHE_HOME="$cache_home"
        mkdir -p "$XDG_CACHE_HOME" 2>/dev/null || true
      fi
    fi
  fi
}

yeban_local_env
