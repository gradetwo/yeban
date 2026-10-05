#!/usr/bin/env bash
# MUST-GATE-009：**发行物层面**的"危险能力默认关"断言。
#
# 规范要求（`gate-status.md` 的 MUST-GATE-009 一行）：进程内已有 112 条判据（默认关 / 只绑环回 /
# 0600 / `ui:inject` 先于 token 硬禁），**缺的是"发行物层面"的断言** —— 也就是：
# **默认构建出来的那个二进制**，到底有没有把网络监听代码链进去？
#
# 本脚本不问源码（源码层的 `default = []` 只证明清单写法），而是**问产物**：
#   ① 默认构建的二进制 + `--enable-mcp-http` ⇒ 必须**拒绝**（因为 `mcp-http` 没编进去）
#      —— 这才是"默认构建里不会链接任何网络监听代码"的**产物级**证据；
#   ② 若二进制确实带了该 feature ⇒ 必须**只绑环回**，并且**必须**给 `--enable-mcp-http` 才允许监听。
#
# 纪律：**工具/产物缺失一律记 `unknown`（exit 2），绝不记 FAIL**（假红会让门禁变成噪音）。
set -uo pipefail
repo_root="$(git rev-parse --show-toplevel)" || exit 2
cd "$repo_root"
if [ -f scripts/dev/local-env.sh ]; then . scripts/dev/local-env.sh >/dev/null 2>&1 || true; fi
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" >/dev/null 2>&1 || true

bin=""
for cand in target/debug/yeban-mcp target/release/yeban-mcp; do
  [ -x "$cand" ] && bin="$cand" && break
done
if [ -z "$bin" ]; then
  if command -v cargo >/dev/null 2>&1; then
    printf '[run] 未找到已构建的 yeban-mcp，尝试 cargo build -p yeban-mcp（默认 feature）\n'
    cargo build -p yeban-mcp >/dev/null 2>&1 && [ -x target/debug/yeban-mcp ] && bin=target/debug/yeban-mcp
  fi
fi
if [ -z "$bin" ]; then
  printf '[unknown] 没有可用的 yeban-mcp 产物（或 cargo 不可用）—— 本环境无法判定 MUST-GATE-009 的产物级断言\n'
  exit 2
fi
printf '[info] 产物: %s\n' "$bin"

fail=0
# ⚠ 第一版这条判据是**空转的**（我自己抓到）: 我传了 `--port 39876`，而该二进制**没有 `--port`**
# ⇒ 它以 rc=2 "未知参数" 退出，我却把"拒绝了"当成"没有链接监听代码"的证据。**任何"因为别的原因失败"
# 都能骗过这种写法。** ⇒ 现在只传**真实存在的** `--enable-mcp-http`（见 `--help`），并要求:
# 非零退出 **且** 输出点名 feature/编译（证明它是因为"没编进来"而拒绝，不是别的原因）。
out=$("$bin" --enable-mcp-http 2>&1 </dev/null | head -c 400); rc=$?
if [ "$rc" -eq 0 ]; then
  printf 'FAIL 默认产物接受了 --enable-mcp-http（rc=0）—— 默认构建里可能真的链进了网络监听代码\n' >&2
  fail=1
elif printf '%s' "$out" | grep -qiE 'mcp-http|feature|未编译|not compiled|需要'; then
  printf '[ok] 默认产物拒绝 --enable-mcp-http 且点名 feature（rc=%s）: %s\n' "$rc" "$(printf '%s' "$out" | head -1)"
elif printf '%s' "$out" | grep -qiE 'Operation not permitted|Permission denied|os error|令牌文件 I/O|只读文件系统'; then
  # ⚠ 环境级失败**不是**缺陷: 例如沙箱不允许写 `~/.yeban/session.token` ⇒ 二进制在走到开关逻辑**之前**就退出了。
  # 这与"产物把监听代码链进来了"是两件事 ⇒ 记 **unknown（exit 2）**，绝不记 FAIL。
  # （实测: 我在受限沙箱里跑这一条时就是 3 号分支 —— 第一版会把它报成 FAIL, 又是一次假红。）
  printf '[unknown] 产物因**环境**原因提前退出（未能走到开关判定）: %s\n' "$(printf '%s' "$out" | head -1)"
  exit 2
else
  printf 'FAIL 默认产物拒绝了该开关, 但**没有**点名 feature —— 无法证明拒绝是因为"没编进来"（可能是别的原因）\n' >&2
  printf '      输出: %s\n' "$out" >&2
  fail=1
fi

# ② 若帮助文本**不**含"需要 --features mcp-http"，说明该产物是带 feature 的构建 ⇒ 再查环回口径
if "$bin" --help 2>&1 | grep -q '需要 --features mcp-http'; then
  printf '[skip] 该产物是**默认构建**（帮助文本自述需要该 feature）—— 这正是默认产物应有的样子\n'
else
  out2=$("$bin" --enable-mcp-http 2>&1 </dev/null | head -c 400)
  printf '[info] 带该 feature 的产物，显式开关下的首行: %s\n' "$(printf '%s' "$out2" | head -1)"
  if printf '%s' "$out2" | grep -qE '0\.0\.0\.0|非环回|non-loopback'; then
    printf 'FAIL 出现非环回绑定痕迹\n' >&2
    fail=1
  fi
fi

[ "$fail" -eq 0 ] && { printf 'check_release_defaults: 通过（MUST-GATE-009 的产物级部分）\n'; exit 0; }
exit 1
