#!/usr/bin/env bash
# 共享 target 的**单写者锁**：把"编译"这一件真正串行的事显式排队，其余一切照旧并行。
#
# ## 为什么需要它（2026-10-25 实测，R1137）
#
# 本仓纪律（AGENTS §5.2）要求**共享 `CARGO_TARGET_DIR`**（私有 target 会重编 Slint，
# 而本机是 M2 开发机）。但共享 target 隐含一个**未被写下来的假设**：同一时刻只有一条
# 线在编译。今夜两条线同时编译时出现过三种症状：
#
#   * `Finished` 之后 test 二进制**被另一个进程删掉** ⇒ `could not execute process`
#   * `yeban-mcp` 读到**别的分支的** `yeban_model` rlib ⇒ `E0599` / `E0004`
#   * `target/debug/.fingerprint/<crate>-<hash>/` 几分钟内**自行消失**
#
# 三种都不是"某个 diff 有问题"，而是**两个 cargo 进程在同一个目录里互相踩**。
# 纪律不能靠自觉 ⇒ 这个脚本把锁变成机械的。
#
# ## 用法
#
#     scripts/dev/target-lock.sh -- cargo test -p yeban-mcp --locked
#     YEBAN_LOCK_TIMEOUT=3600 scripts/dev/target-lock.sh -- bash scripts/gates/run-gates.sh light
#
# 拿不到锁就一直等（默认 2 小时超时）。**持锁期间**才跑真正的编译/测试命令。
# 只读命令（`git`、`grep`、`python3 scripts/guards/`）**不需要**这把锁。
#
# ## 实现
#
# `mkdir` 是原子的（POSIX），而 macOS 默认没有 `flock(1)` ⇒ 用 `mkdir` 当自旋锁。
# 锁目录里写 pid + hostname + 起始时间，便于判断"持锁者是不是已经死了"：
# 同一台机器上持锁进程不存在 ⇒ 判定为陈旧锁并夺回（⛔ 不静默，打印一行告警）。
set -uo pipefail

LOCK="${CARGO_TARGET_DIR:-target}/.yeban-build.lock"
TIMEOUT="${YEBAN_LOCK_TIMEOUT:-7200}"
POLL="${YEBAN_LOCK_POLL:-5}"
# 单次持有的**上限**。超过它即判定为孤儿并**响亮**回收。
#
# ⚠ 为什么需要它（2026-10-25 实测教训）：我为了解一个"看起来卡住"的锁**手删了锁目录**，
# 而那个锁当时**真的被持有** ⇒ 造成**两写者窗口**（同时 2 个 `cargo test` ＋ 2 个
# `cargo check` 打同一个 target）⇒ 只好杀掉所有构建收场。正确的解不是手删，而是
# **让工具自己有界**：任何一次持有都不该超过这个上限（今夜最长的合法持有约 20 分钟），
# 超了就当作孤儿，**打印持锁者的完整信息**再夺回。
MAX_HOLD="${YEBAN_LOCK_MAX_HOLD:-3600}"

if [ "${1:-}" = "--" ]; then
  shift
fi
if [ "$#" -eq 0 ]; then
  echo "用法: scripts/dev/target-lock.sh -- <命令...>" >&2
  exit 2
fi

mkdir -p "$(dirname "$LOCK")" || exit 1

acquired=0
start=$(date +%s)
while :; do
  if mkdir "$LOCK" 2>/dev/null; then
    acquired=1
    printf 'pid=%s\nhost=%s\nsince=%s\nsince_epoch=%s\ncmd=%s\n' \
      "$$" "$(hostname 2>/dev/null || echo unknown)" \
      "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(date +%s)" "$*" >"$LOCK/owner" 2>/dev/null || true
    break
  fi
  # 陈旧锁回收：持锁方是同机且 pid 已不存在 ⇒ 夺回。
  owner_pid=$(sed -n 's/^pid=//p' "$LOCK/owner" 2>/dev/null | head -1)
  owner_host=$(sed -n 's/^host=//p' "$LOCK/owner" 2>/dev/null | head -1)
  my_host=$(hostname 2>/dev/null || echo unknown)
  if [ -n "$owner_pid" ] && [ "$owner_host" = "$my_host" ] && ! kill -0 "$owner_pid" 2>/dev/null; then
    echo "[target-lock] ⚠ 回收陈旧锁：持锁 pid=$owner_pid 已不存在（$(cat "$LOCK/owner" 2>/dev/null | tr '\n' ' ')）" >&2
    rm -rf "$LOCK" 2>/dev/null || true
    continue
  fi
  now=$(date +%s)
  # 有界持有：超过 MAX_HOLD 一律当孤儿**响亮**回收（见文件头那段实测教训）
  held_since=$(sed -n 's/^since_epoch=//p' "$LOCK/owner" 2>/dev/null | head -1)
  if [ -n "$held_since" ] && [ $((now - held_since)) -ge "$MAX_HOLD" ]; then
    echo "[target-lock] ⚠ 回收超期锁：已持有 $((now - held_since))s ≥ 上限 ${MAX_HOLD}s；持锁者 $(cat "$LOCK/owner" 2>/dev/null | tr '\n' ' ')" >&2
    rm -rf "$LOCK" 2>/dev/null || true
    continue
  fi
  if [ $((now - start)) -ge "$TIMEOUT" ]; then
    echo "[target-lock] ⛔ 等锁超时（${TIMEOUT}s）：持锁者 $(cat "$LOCK/owner" 2>/dev/null | tr '\n' ' ')" >&2
    exit 75
  fi
  sleep "$POLL"
done

cleanup() {
  if [ "$acquired" = "1" ]; then
    rm -rf "$LOCK" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

"$@"
rc=$?
exit "$rc"
