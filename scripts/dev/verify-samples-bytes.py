#!/usr/bin/env python3
"""校验 `assets/samples/manifest.json` 里**已落地**的资产字节是否与登记一致。

为什么需要它（第 43 轮 needs N3）: 那张清单是 8.9 MB 的**生成物**，而生成器/校验器此前只以
**附录**形式嵌在一条线的台账里 —— **生成物没有生成器、登记没有校验器**，正是本项目最不该留的形态
（"登记义务已完成"只有在**能被复核**时才成立）。`validate_schemas.py --repo-assets` 已经在**契约层**
对账，但它与 cwd/网络无关地只校验**磁盘上存在的**文件；本脚本把"哪些是 optional 未分发、哪些真的缺、
哪些字节不符"分门别类讲清楚，并可作手动档联网校验的入口。

用法:
    python3 scripts/dev/verify-samples-bytes.py                 # 报告 + 缺件/不符则非零退出
    python3 scripts/dev/verify-samples-bytes.py --expect-missing # 允许 optional 全部缺失（登记式入库的当前形态）
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = REPO / "assets" / "samples" / "manifest.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--expect-missing",
        action="store_true",
        help="optional=true 的资产允许不在磁盘上（当前登记式入库的形态）",
    )
    args = parser.parse_args()

    if not MANIFEST.is_file():
        print(f"没有 {MANIFEST.relative_to(REPO)} —— 素材尚未登记", file=sys.stderr)
        return 2

    doc = json.loads(MANIFEST.read_text(encoding="utf-8"))
    items = doc.get("items", [])
    present = mismatched = missing_optional = missing_required = 0
    problems: list[str] = []

    for item in items:
        rel = item.get("relative_path", "")
        target = REPO / rel
        if not target.is_file():
            if item.get("optional") is True:
                missing_optional += 1
            else:
                missing_required += 1
                problems.append(f"{item.get('id')}: 声明为**应分发**却不在磁盘上: {rel}")
            continue
        present += 1
        digest = hashlib.sha256(target.read_bytes()).hexdigest()
        if digest != item.get("sha256"):
            mismatched += 1
            problems.append(
                f"{item.get('id')}: sha256 不符 {rel}\n"
                f"      登记 {item.get('sha256')}\n      实际 {digest}"
            )
        elif target.stat().st_size != item.get("size_bytes"):
            mismatched += 1
            problems.append(f"{item.get('id')}: size_bytes 不符 {rel}")

    print(
        f"assets/samples/manifest.json: 登记 {len(items)} 项 | 磁盘上存在 {present} | "
        f"optional 未分发 {missing_optional} | 应分发却缺 {missing_required} | 字节不符 {mismatched}"
    )
    if present == 0:
        print(
            "  ⚠ 磁盘上**一个资产字节都没有** ⇒ 本次校验只证明清单自洽，**没有**校验任何素材内容。\n"
            "    这是登记式入库（`optional: true`）的**代价**，不是通过。"
        )
    for problem in problems:
        print(f"  ✗ {problem}", file=sys.stderr)

    if problems:
        return 1
    if missing_optional and not args.expect_missing and present == 0:
        # 登记式入库是当前**有意**的形态（清单入库存根、字节待落地）⇒ 明确说明而不是含糊通过。
        print("  （当前为登记式入库：字节未随仓库分发。素材落地后本脚本即升级为真字节校验，无需改代码。）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
