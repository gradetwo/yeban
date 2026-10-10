#!/usr/bin/env python3
"""R65: 工作流结构自检 —— 工作流的改动必须至少被一条自动步骤验证。

动机 (R65): 一次只改 `.github/workflows/ci.yml` + 文档的推送会让 `plan` 判出空
crate 集合, 于是 `rust` 矩阵腿**整体 skipped**, 新加的腿在它自己的提交上一次都
没跑。因此本守卫**不依赖 crate 集合**: 只要 `checks` job 跑, 它就验证本文件可
解析、且关键腿与它们的守卫在位。

同时守住 R64: `yeban-render` 的 `src/logic.rs` (8252 行 / 37 判据) 与
`src/als.rs` (1587 行 / 5 判据) 曾被 feature 门挡在自动门禁之外 —— 那两条腿
一旦被删或丢了 `--features` 开关, 它们会静默退回"从不编译"。
"""

from __future__ import annotations

import pathlib
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"

# R64 的两条腿: 名字里带这个子串, 且 run 里必须同时出现 Cargo.toml 守卫与 --features。
EXPORT_LEG_MARKER = "导出特性"
REQUIRED_IN_RUN = (
    "experimental-logic-export",  # Cargo.toml 守卫 + 特性开关
    "experimental-als-export",  # 第二个特性开关
)


def fail(msg: str) -> None:
    print(f"[FAIL] {msg}")
    sys.exit(1)


def main() -> int:
    try:
        doc = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001
        fail(f"{WORKFLOW} 无法解析: {exc}")

    jobs = (doc or {}).get("jobs") or {}
    if not jobs:
        fail("ci.yml 没有任何 job")

    for name, job in jobs.items():
        if not isinstance(job, dict):
            fail(f"job {name} 不是映射")
        # uses-reusable / 只有 runs-on 的占位 job 也要求 steps 非空。
        if not job.get("runs-on"):
            fail(f"job {name} 缺少 runs-on")
        if not job.get("steps"):
            fail(f"job {name} 缺少 steps")

    rust = jobs.get("rust")
    if not rust:
        fail("ci.yml 缺少 rust 矩阵 job")
    steps = rust.get("steps") or []
    legs = [s for s in steps if EXPORT_LEG_MARKER in str(s.get("name", ""))]
    if len(legs) != 2:
        names = [s.get("name") for s in legs]
        fail(
            "实验性导出腿的数量不是 2 (R64): "
            f"{names} —— 少了就会让 logic.rs / als.rs 退回'从不编译'"
        )
    for step in legs:
        run = str(step.get("run", ""))
        for needle in REQUIRED_IN_RUN:
            if needle not in run:
                fail(f"腿 {step.get('name')!r} 的 run 里缺少 {needle!r}")

    # 平台腿是 R60/R63 的受害面: 它必须存在且有自己的步骤, 否则换行锚点类缺陷
    # 只会在 Linux 检出上被验证。
    windows = jobs.get("windows")
    if not windows:
        fail("ci.yml 缺少 windows 平台腿 (R60/R63 的验证面)")
    if not windows.get("steps"):
        fail("windows 腿缺少 steps")

    print(
        f"[ok] ci.yml 结构自检通过: {len(jobs)} 个 job, "
        f"rust job {len(steps)} 个 step, 实验性导出腿 {len(legs)} 条, "
        f"windows 腿 {len(windows['steps'])} 个 step"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
