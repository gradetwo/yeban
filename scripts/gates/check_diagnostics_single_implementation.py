#!/usr/bin/env python3
"""`[D56 判据 4]` 诊断导出的**两个入口必须共用同一实现**。

为什么机械化: D56 要求 UI 动作与 MCP 工具都调 `yeban_diagnostics::export_diagnostics`。
"打算共用"不算共用 —— 只要有人把打包逻辑复制一份, 这条就红。判据如下:

1. UI 侧 (`crates/yeban-app/src/undo.rs`) 必须真的调用共享函数;
2. MCP 侧 (`crates/yeban-mcp/src/domain/diagnostics.rs`) 必须真的调用共享函数;
3. **只有**共享 crate 可以自己造 zip writer (`zip::ZipWriter`); 两个入口各自造一个就是两份实现。
"""
from __future__ import annotations

import pathlib
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
SHARED = "yeban_diagnostics::export_diagnostics"
ENTRY_POINTS = {
    "UI 动作": REPO / "crates/yeban-app/src/undo.rs",
    "MCP 工具": REPO / "crates/yeban-mcp/src/domain/diagnostics.rs",
}
COLLECTOR = REPO / "crates/yeban-diagnostics/src/lib.rs"


def main() -> int:
    problems: list[str] = []
    for label, path in ENTRY_POINTS.items():
        if not path.is_file():
            problems.append(f"{label}: 找不到 {path.relative_to(REPO)}")
            continue
        text = path.read_text(encoding="utf-8")
        if SHARED not in text:
            problems.append(f"{label}（{path.relative_to(REPO)}）没有调用共享实现 `{SHARED}`")
        if "ZipWriter" in text:
            problems.append(f"{label}（{path.relative_to(REPO)}）自己造了 zip writer ⇒ 这是第二份实现")
    if not COLLECTOR.is_file():
        problems.append("找不到共享采集器 crates/yeban-diagnostics/src/lib.rs")
    elif "ZipWriter" not in COLLECTOR.read_text(encoding="utf-8"):
        problems.append("共享采集器里竟然没有 zip writer ⇒ 判据的前提被改动了, 请复核本判据")
    if problems:
        print("[FAIL] D56 判据 4（两入口共用同一实现）未通过:", file=sys.stderr)
        for p in problems:
            print(f"       - {p}", file=sys.stderr)
        return 1
    print("[ok] D56 判据 4: UI 动作与 MCP 工具都调用 yeban_diagnostics::export_diagnostics, 且只有共享 crate 造 zip")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
