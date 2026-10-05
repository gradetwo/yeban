#!/usr/bin/env python3
"""测量一个进程的**峰值常驻内存**（RSS），服务于 `BASELINE-002`。

规范（`docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md`）：
「空工程空闲常驻内存 | 目标 **≤ 35 MB**」。此前它是 **PENDING**：仓库里**没有任何内存测量**。

# 为什么住在 `scripts/` 而不是某个 crate 的 example

① 它要测的是**任意进程**（今天测 `yeban-app --headless`，明天测 MCP 服务、或某个基准二进制），
   放进某一个 crate 会把"测量"和"被测对象"绑在一起；
② 它必须能在**没有工具链改动**的前提下被 CI 调用（`.github/**` 与 `scripts/**` 都是集成者地盘）；
③ 读 RSS 用 **`resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`**（Python 标准库）：
   它是被 `wait()` 过的子进程的**峰值 RSS 高水位**，**不需要读别的进程、不需要 `ps`**、
   不引入依赖、不写 `unsafe`。实测教训：第一版用 `ps -o rss= -p <pid>`，
   在受沙箱限制的环境里直接 `Operation not permitted` —— 而 `getrusage` 没有这个限制。
   ⚠ `ru_maxrss` 的单位**按平台不同**（Linux = KB，macOS = 字节），本脚本按平台归一为 MB。

# 诚实边界（**必须**与读数一起引用）

- 这里读的是**峰值 RSS**，不是"稳定空闲值"；对短命进程（例如 `--headless` 自检）二者接近，
  对长命进程应当在 `--settle-seconds` 之后再开始采样。
- **托管 runner 的绝对值只作数量级参考**：真正的"≤35 MB"要在规范指定的参考机上复跑同一条命令。
- RSS 包含动态库映射带来的页，跨平台/跨 libc 不可直接相比。

用法：
    python3 scripts/gates/measure_rss.py --label app-headless -- \\
        ./target/release/yeban-app --headless
    python3 scripts/gates/measure_rss.py --label mcp-server --settle-seconds 3 -- \\
        ./target/release/yeban-mcp
"""

from __future__ import annotations

import argparse
import resource
import subprocess
import sys
import time
from pathlib import Path

#: 规范目标（MB）。
TARGET_MB = 35.0


def peak_rss_mb() -> float:
    """被 `wait()` 过的子进程的峰值 RSS（MB）。

    `RUSAGE_CHILDREN.ru_maxrss` 是**高水位**而不是累计值 ⇒ 本脚本**每次只量一条命令**
    （多次测量请分多次调用；这也是它被设计成"一条命令一个进程"的原因）。
    """
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    raw = float(usage.ru_maxrss)
    # Linux 报 KB，macOS 报字节。
    if sys.platform == "darwin":
        return raw / (1024.0 * 1024.0)
    return raw / 1024.0


def main() -> int:
    parser = argparse.ArgumentParser(description="峰值 RSS 测量（BASELINE-002）")
    parser.add_argument("--label", required=True, help="读数标签（进 BENCH 行）")
    parser.add_argument("--settle-seconds", type=float, default=0.0, help="启动后先等这么久再采样")
    parser.add_argument("--timeout", type=float, default=600.0, help="被测命令的总超时（秒）")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="-- 之后的被测命令")
    args = parser.parse_args()

    command = [item for item in args.command if item != "--"]
    if not command:
        print("用法: measure_rss.py --label NAME -- <command...>", file=sys.stderr)
        return 2

    started = time.monotonic()
    process = subprocess.Popen(command)
    try:
        if args.settle_seconds > 0:
            time.sleep(min(args.settle_seconds, args.timeout))
        code = process.wait(timeout=max(1.0, args.timeout - (time.monotonic() - started)))
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        print(f"[measure_rss] 超时 {args.timeout}s，已终止被测进程", file=sys.stderr)
        return 3
    except KeyboardInterrupt:
        process.kill()
        process.wait()
        raise
    peak_mb = peak_rss_mb()

    # 命令本身的退出码**必须**透传：否则"测到了内存"会掩盖"被测命令其实失败了"。
    verdict = "within-target" if 0 < peak_mb <= TARGET_MB else "over-target"
    if peak_mb <= 0:
        verdict = "unmeasured"
    print(
        f"BENCH baseline=002 label={args.label} peak_rss_mb={peak_mb:.2f} "
        f"target_mb={TARGET_MB:.1f} child_exit={code} verdict={verdict}"
    )
    print(
        'BENCH baseline=002 note="RSS 取自 resource.getrusage(RUSAGE_CHILDREN).ru_maxrss; '
        '达标需在规范指定参考机复跑同一命令; 详细边界见 scripts/gates/measure_rss.py 的文件头"'
    )
    if peak_mb <= 0:
        print("[measure_rss] 拿不到 RSS（本平台的 ru_maxrss 语义可能不同）", file=sys.stderr)
        return 4
    return code


if __name__ == "__main__":
    raise SystemExit(main())
