#!/usr/bin/env python3
"""判断某个成员 crate 是否**传递地**含"本机不该编译的重依赖"。

为什么需要一条独立脚本（而不是在 `run-gates.sh` 里 grep）：
人类负责人的 CI/CD 纪律是「**除非特殊情况或只能本机测，不要在本地跑长耗时/高耗 CPU 的活**」，
所以"本机跳过哪些 crate"必须由**依赖图**决定，而不是由某个 crate 自己的清单决定。

⚠ 这里踩过一次真实的坑：`yeban-mcp` 自己的 `Cargo.toml` 里**没有任何**重依赖，
但它依赖 `yeban-render` ⇒ 传递拉进 `rayon`/`hound`/`midly`。
只看本 crate 清单的实现会**在本机真的编译它们**，直接违背上述纪律。
（由 `line/mcp-render` 的 needs-7 发现。）

用法: `python3 scripts/dev/heavy-deps.py <crate>`。
退出码: `0` = 含重依赖（本机应跳过）；`1` = 不含（本机可跑）；`2` = **无法判定**（本机也跳过，但要说清原因）。
"""

from __future__ import annotations

import json
import subprocess
import sys

#: 这些依赖一旦出现在**传递闭包**里，就认为本机不该编译（CI 在 GitHub 上做）。
HEAVY = (
    "slint",
    "cpal",
    "winit",
    "symphonia",
    "rubato",
    "rayon",
    "hound",
    "midly",
)


def metadata() -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def reachable(members: dict[str, dict], start: str) -> set[str]:
    """成员之间的传递闭包（只走 workspace 内部依赖边）。"""
    seen: set[str] = set()
    stack = [start]
    while stack:
        name = stack.pop()
        if name in seen or name not in members:
            continue
        seen.add(name)
        for dep in members[name].get("dependencies", []):
            if dep["name"] in members and dep["name"] not in seen:
                stack.append(dep["name"])
    return seen


def main() -> int:
    if len(sys.argv) != 2:
        print("用法: heavy-deps.py <crate>", file=sys.stderr)
        return 2
    crate = sys.argv[1]
    try:
        data = metadata()
    except (subprocess.CalledProcessError, FileNotFoundError) as error:
        # 拿不到依赖图时**保守**返回"无法判定 ⇒ 本机跳过"（宁可本机少跑, 也不违背纪律）。
        print(f"无法读取依赖图({error.__class__.__name__}): 保守跳过", file=sys.stderr)
        return 2
    members = {pkg["name"]: pkg for pkg in data.get("packages", [])}
    if crate not in members:
        print(f"{crate} 不在 workspace 成员里", file=sys.stderr)
        return 2
    closure = reachable(members, crate)
    names: set[str] = set()
    for name in closure:
        names.update(dep["name"] for dep in members[name].get("dependencies", []))
    hits = sorted(names.intersection(HEAVY))
    if hits:
        print(f"{crate}: 传递含重依赖 {', '.join(hits)}（闭包 {len(closure)} 个成员）")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
