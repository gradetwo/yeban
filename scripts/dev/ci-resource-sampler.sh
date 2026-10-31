#!/usr/bin/env bash
# CI 资源采样器：每隔 N 秒往日志里打**一行**盘 + 内存读数。
#
# ## 为什么需要它（不是"顺手加个 log"）
#
# `ci.yml` 的 `rust (workspace 全量)` 腿里，`test (合并特性, yeban-app)` 这一步连着两次
# 以同一句收场（run `41057489656`／`41058208805`，都在该步起跑后约 4 分钟、4 次链接各一次）：
#
#     collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped
#
# 本仓库**已有**的解释是"爆盘"（见 ci.yml 回收步骤里 `5dd70785` 那次实测：ENOSPC ＋ SIGBUS
# 同现）。但这两次不同：日志里 `No space left on device` **0 行**（单位＝行，`grep -c` 现读），
# 而这一步开跑**前一行**的 `df -h .` 是 `145G 99G 46G 69%`。⇒ 盘满解释**未被证明**。
# SIGBUS 也可能是内存/换页或 lld 自身崩溃，而现有日志里没有任何读数能分辨这三类。
#
# 靠 rerun 猜一次要 30 分钟 CI；打点一次只要几十行日志。下一次复现时，判"是哪一类"
# 应当能从**同一份日志**读出来，而不是再从症状推断。
#
# ## 口径（每个数字在数什么，先说清）
#
# - `disk_avail` ＝ `df -BG /` 第 4 列，单位 **GiB**，根文件系统当前可用；
# - `ram_avail` ＝ `/proc/meminfo` 的 `MemAvailable`，单位 **MiB**（内核给出的"还能借多少"，
#   比 `free` 第 4 列的 `MemFree` 更贴近可用量）；
# - `swap_avail` ＝ `/proc/meminfo` 的 `SwapFree`，单位 **MiB**（还剩多少可换页）；
# - `ld_n` ＝ 当前进程名恰为 `ld`/`ld.lld`/`collect2` 的**进程数**（数进程，不数行）。
#
# 用法（CI 里）：
#
#     bash scripts/dev/ci-resource-sampler.sh &
#     SAMPLER=$!
#     trap 'kill "$SAMPLER" 2>/dev/null || true' EXIT
#
# 默认每 20 秒一行、共 120 行（=40 分钟），之后自己退出 —— 即使调用方忘了 kill，也不会
# 留下无限打印的后台作业。

set -u

interval="${1:-20}"
ticks="${2:-120}"

for _ in $(seq 1 "$ticks"); do
    disk_avail="$(df -BG / 2>/dev/null | awk 'NR==2 {print $4}')"
    ram_avail="$(awk '/^MemAvailable:/{print int($2/1024)}' /proc/meminfo 2>/dev/null)"
    swap_avail="$(awk '/^SwapFree:/{print int($2/1024)}' /proc/meminfo 2>/dev/null)"
    ld_n=0
    for name in ld ld.lld collect2; do
        ld_n=$((ld_n + $(pgrep -x "$name" 2>/dev/null | wc -l | tr -d ' ')))
    done
    printf '[sampler] %s disk_avail=%s ram_avail=%sMiB swap_avail=%sMiB ld_procs=%s\n' \
        "$(date -u +%H:%M:%S)" "${disk_avail:-?}" "${ram_avail:-?}" "${swap_avail:-?}" "$ld_n"
    sleep "$interval"
done
