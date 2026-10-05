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
    # 泳道循环住在**卷帘**里, 不在 app.slint 里（守卫第一次写成搜错文件, 被自己的 FAIL 抓住）。
    roll = (root / "crates/yeban-app/ui/console/piano_roll.slint").read_text(encoding="utf-8")
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
    # 第五处：宿主常量 `ROLL_LANE_COUNT` 必须等于 `.slint` 里画出的泳道数。
    # 为什么：两处各自是"真相", 漂移后宿主发布的音高上下界会与实际画出的泳道**不一致**,
    # 而这种错在界面上看起来只是"少画了几条", 不会被任何既有判据抓住（账本第 202/462 轮）。
    import re

    m_slint = re.search(r"for lane_index in (\d+)", roll)
    m_host = re.search(r"const ROLL_LANE_COUNT: i32 = (\d+);", host)
    if m_slint is None:
        problems.append("piano_roll.slint 里找不到 `for lane_index in <N>` ⇒ 本判据失效, 需更新")
    elif m_host is None:
        problems.append("host.rs 里找不到 `const ROLL_LANE_COUNT: i32 = <N>;`")
    elif m_slint.group(1) != m_host.group(1):
        problems.append(
            f"泳道数不一致: .slint 画 {m_slint.group(1)} 条, 宿主常量是 {m_host.group(1)}"
            " ⇒ 发布的音高上下界与实际绘制不符"
        )
    else:
        lane_ok = m_slint.group(1)

    if problems:
        print("[FAIL] viewport-bounds-wiring:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print(
        f"[ok] viewport-bounds-wiring: {len(BOUNDS)} 个视口边界在三处齐全,"
        f"泳道数一致 ({lane_ok})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
