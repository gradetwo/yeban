#!/usr/bin/env python3
"""钉住 `yeban-mcp` 的**依赖方向**规则。

`crates/yeban-mcp/Cargo.toml` 自己写着"**轻量 crate**: 不拖音频栈进 MCP"，而本会话曾两次
差点违反它（想给 MCP 加 `yeban-app` 以取音符构造；想加 `yeban-render` 以取 SMF 编码器）。
规则写在注释里**挡不住**下一次，所以要有一个会失败的检查（账本第 261/273 轮）。

判据：`[dependencies]` 段里出现任何被禁 crate ⇒ 非零退出并指名。
"""
from __future__ import annotations

import pathlib
import sys

MANIFEST = pathlib.Path("crates/yeban-mcp/Cargo.toml")

#: 被禁的直接依赖：带 GUI / 音频栈 / 渲染栈的 crate。MCP 要这些东西时，
#: 正确做法是**把共享件下移到 model 级 crate**，而不是加依赖（账本第 261 轮）。
FORBIDDEN = (
    "yeban-app",      # 带 GUI（Slint）
    "yeban-dsp",      # 音频栈
    "yeban-render",   # 依赖 dsp + hound
    "yeban-engine",   # 设备/回调
    "yeban-audio",    # 音频 I/O
)


def main() -> int:
    text = MANIFEST.read_text(encoding="utf-8")
    # 只看 [dependencies] 段（到下一个 [section] 为止）
    start = text.find("[dependencies]")
    if start < 0:
        print("[FAIL] 找不到 [dependencies] 段", file=sys.stderr)
        return 1
    rest = text[start + len("[dependencies]"):]
    end = rest.find("\n[")
    block = rest if end < 0 else rest[:end]

    # ⚠ 必须**逐行取依赖键**：第一版用子串匹配, 命中了段内**注释**里提到的 crate ⇒ 干净树也报错
    # （牙测当场暴露）。这里去掉 `#` 之后的注释, 只认 `name = ...` / `name.workspace = ...` 的键。
    keys = set()
    for raw in block.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line or "=" not in line:
            continue
        key = line.split("=", 1)[0].strip()
        keys.add(key)
    hits = [name for name in FORBIDDEN if name in keys]
    if hits:
        print(f"[FAIL] yeban-mcp 直接依赖了被禁 crate: {', '.join(hits)}", file=sys.stderr)
        print("       规则见 crates/yeban-mcp/Cargo.toml 的'轻量 crate: 不拖音频栈进 MCP'；", file=sys.stderr)
        print("       正确做法是把共享件下移到 model 级 crate（账本第 261/273 轮）。", file=sys.stderr)
        return 1
    print(f"[ok] mcp-dependency-direction: {len(FORBIDDEN)} 个被禁 crate 均未被直接依赖")
    return 0


if __name__ == "__main__":
    sys.exit(main())
