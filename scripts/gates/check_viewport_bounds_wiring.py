#!/usr/bin/env python3
"""`[UI-NOTE-001]` 步骤 ① 的三方一致性守卫。

规范要求视口以 `min_tick` / `max_tick` / `min_pitch` / `max_pitch` 暴露给裁剪核心。
本守卫断言四个边界**三处都在**，缺一处即失败：

1. `ui/app.slint`：声明为 `in-out property <int> <name>`；(界面契约)
2. `src/host.rs`：有 `set_<name>` 调用；(宿主真的写了)
3. `src/test_port_adapter.rs`：有 `get_<name>` 读取。(有判据在读, 不是"写了没人验证")

为什么需要它: 第 204 轮实测出现过"属性写了但**无人读取**"的状态 —— 那种缺口不会被任何
既有判据抓住, 因为两边单独看都"正常"。
"""

from __future__ import annotations

import pathlib
import sys

BOUNDS = ("roll-min-tick", "roll-max-tick", "roll-min-pitch", "roll-max-pitch")


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[2]
    slint = (root / "crates/yeban-app/ui/app.slint").read_text(encoding="utf-8")
    host = (root / "crates/yeban-app/src/host.rs").read_text(encoding="utf-8")
    crit = (root / "crates/yeban-app/src/test_port_adapter.rs").read_text(encoding="utf-8")
    problems: list[str] = []
    for name in BOUNDS:
        if f"in-out property <int> {name}:" not in slint:
            problems.append(f"{name}: app.slint 未声明为 in-out property <int>")
        setter = "set_" + name.replace("-", "_")
        if f"{setter}(" not in host:
            problems.append(f"{name}: host.rs 没有 {setter}( 调用 ⇒ 宿主没写它")
        getter = "get_" + name.replace("-", "_")
        if f"{getter}(" not in crit:
            problems.append(f"{name}: 没有任何判据读它 ({getter}) ⇒ 写了没人验证")
    if problems:
        print("[FAIL] viewport-bounds-wiring:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print(f"[ok] viewport-bounds-wiring: {len(BOUNDS)} 个视口边界在三处齐全")
    return 0


if __name__ == "__main__":
    sys.exit(main())
