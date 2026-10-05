#!/usr/bin/env python3
"""`docs/ledger/handoff-snapshot.md` 必须与生成器输出**逐字节一致**。

为什么: 那份快照是**生成物**, 手改它、或改了台账却忘记重跑生成器, 都会让它与事实脱节 ——
而它存在的全部意义就是"不让汇总数字悄悄漂移"。实测教训（第 127 轮）: 我在报告里重复了约二十轮的
"18 已接线 / 0 部分", 而表里其实是 17 / 1; 正是这份快照第一次生成时抓到了这个偏差。
所以现在把它变成**机械判据**: 生成 → 比对 → 不一致就红, 并告诉人跑哪条命令。

退出码: 0 = 一致; 1 = 不一致或生成失败。
"""
from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[2]
GEN = REPO / "scripts" / "dev" / "render-handoff.py"
COMMITTED = REPO / "docs" / "ledger" / "handoff-snapshot.md"


def main() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        out = pathlib.Path(tmp) / "handoff-snapshot.md"
        env = dict(os.environ, HANDOFF_OUT=str(out))
        proc = subprocess.run(
            [sys.executable, str(GEN)], cwd=REPO, env=env, capture_output=True, text=True
        )
        if proc.returncode != 0 or not out.exists():
            print(f"[FAIL] 生成交接快照失败: {proc.stderr.strip()[:200]}", file=sys.stderr)
            return 1
        fresh = out.read_bytes()
    if not COMMITTED.exists():
        print("[FAIL] 缺少 docs/ledger/handoff-snapshot.md", file=sys.stderr)
        return 1
    if COMMITTED.read_bytes() != fresh:
        print(
            "[FAIL] docs/ledger/handoff-snapshot.md 与生成器输出不一致 —— 重新生成:\n"
            "       python3 scripts/dev/render-handoff.py",
            file=sys.stderr,
        )
        return 1
    print("[ok] 交接快照与生成器输出逐字节一致")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
