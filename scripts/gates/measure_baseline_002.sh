#!/usr/bin/env bash
# BASELINE-002 的**规范口径**测量：**空**工程 + **空闲常驻**，且读数覆盖 Slint 运行时。
#
# 为什么需要它: `--headless` 走批处理路径，**一个 Slint 对象都不构造**（本机实测 10.05 MB），
# 而规范说的是「空工程**空闲常驻**内存 ≤ 35 MB」—— 只报 `--headless` 等于量了进程骨架。
# `--headless-idle` 才真的构造 `MainWindow` 并逐行光栅化一帧（零新增依赖，`MinimalSoftwareWindow`/`SoftwareRenderer`）。
#
# 用法（**本机可复跑**，这是它存在的意义）:
#   scripts/gates/measure_baseline_002.sh -- /path/to/yeban-app          # 用已建好的二进制（debug 也可以）
#   scripts/gates/measure_baseline_002.sh -- cargo run --release -p yeban-app --locked --
# 环境变量: IDLE_SECONDS（默认 5）、TIMEOUT（默认 300，交给 measure_rss.py）
#
# 退出码: 0 = 四条测量都跑完（**不代表达标** —— 达标需在规范指定参考机复跑）; 1 = 有测量失败; 2 = 用法错误。
set -euo pipefail

IDLE_SECONDS="${IDLE_SECONDS:-5}"
TIMEOUT="${TIMEOUT:-300}"

if [ "$#" -eq 0 ]; then
  echo "用法: $0 -- <yeban-app 命令...>" >&2
  exit 2
fi
if [ "$1" = "--" ]; then shift; fi
if [ "$#" -eq 0 ]; then
  echo "用法: $0 -- <yeban-app 命令...>（'--' 之后不能为空）" >&2
  exit 2
fi

run() {
  local label="$1"; shift
  echo "--- $label ---"
  python3 scripts/gates/measure_rss.py --label "$label" --timeout "$TIMEOUT" -- "$@"
}

echo "### BASELINE-002 峰值常驻内存（目标 ≤ 35 MB）"
echo "app 命令: $*"
echo "idle-seconds: $IDLE_SECONDS"

# ① 进程地板：空工程 + 零 Slint 对象
run app-empty-headless "$@" --headless --project-sample empty
# ② **规范所指的对象**：空工程 + 真 MainWindow（逐行光栅化一帧）后空闲
run app-empty-headless-idle "$@" --headless-idle --idle-seconds "$IDLE_SECONDS" --project-sample empty
# ③ 对照：6 轨演示工程 ⇒ 证明读数**随对象变化**（不是常数、不是仪器地板）
run app-demo-headless-idle "$@" --headless-idle --idle-seconds "$IDLE_SECONDS" --project-sample default

# ④ 见证：证明 ② 真的建了控件树（不是空转）—— 供人读，且不因 grep 无命中而失败
echo "--- 见证（windows-created / rendered / 像素 / 颜色 / elements） ---"
"$@" --headless-idle --idle-seconds 1 --project-sample empty 2>&1 \
  | grep -E 'headless-idle-witness|view-counts' || true

echo
echo "**诚实边界**：读数取自 \`resource.getrusage(RUSAGE_CHILDREN).ru_maxrss\`（子进程峰值高水位）；"
echo "托管 runner 的绝对值只能给数量级，**达标判定必须在规范指定的参考机上复跑同一条命令**。"
