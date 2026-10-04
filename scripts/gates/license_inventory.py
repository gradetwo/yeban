#!/usr/bin/env python3
"""依赖许可清单生成与漂移检查 (dependency licence inventory)。

为什么需要它（AGENTS.md §2 红线 2：依赖许可红线）：

- `cargo deny check`（MUST-GATE-004）审的是**策略**：白名单之外的许可一律拒绝。
  它不会告诉你"现在到底依赖了哪些包、各是什么许可"。
- 而"源码级移植"（例如从 `synth-core` 移植 DSP）**完全不在** `cargo deny` 的视野里 ——
  那类归属只能靠 `THIRD_PARTY_LICENSES.md` 这类文档承载。文档一旦与真实依赖图脱节，
  合规审计就变成了读故事。

所以这里把"真实依赖图 → 机器生成的清单"这条路固化下来：

    python3 scripts/gates/license_inventory.py            # 重新生成清单
    python3 scripts/gates/license_inventory.py --check    # 与已提交的清单对账（CI 用）

`--check` 会变红的场景：有人加了依赖/升了版本却没重新生成清单（**文档漂移**），
或者有人手工改动了清单（**清单不再是实测结果**）。生成物里记录了 Cargo.lock 的 SHA-256，
因此"清单对应哪一版依赖图"是可核验的，而不是靠日期猜。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "docs" / "ledger" / "dependency-licenses.md"
LOCK = REPO / "Cargo.lock"


def lock_digest() -> str:
    return hashlib.sha256(LOCK.read_bytes()).hexdigest()[:16] if LOCK.is_file() else "no-lock"


def metadata() -> dict:
    proc = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        print("cargo metadata --locked 失败：", file=sys.stderr)
        print(proc.stderr, file=sys.stderr)
        if "Operation not permitted" in proc.stderr or "not installed" in proc.stderr:
            print(
                "提示: 受限沙箱里先加载本机环境 (scripts/dev/local-env.sh)，"
                "或用 `bash scripts/gates/run-gates.sh light` 让门禁脚本处理：\n"
                "  source scripts/dev/local-env.sh",
                file=sys.stderr,
            )
        raise SystemExit(2)
    return json.loads(proc.stdout)


def direct_usage(meta: dict) -> dict[str, set[str]]:
    """哪些 workspace 成员**直接**依赖了这个包（用于区分直接/传递依赖）。"""
    members = {p["id"]: p["name"] for p in meta["packages"] if p["source"] is None}
    usage: dict[str, set[str]] = {}
    for pkg in meta["packages"]:
        if pkg["id"] not in members:
            continue
        for dep in pkg.get("dependencies", []):
            usage.setdefault(dep["name"], set()).add(members[pkg["id"]])
    return usage


def render(meta: dict) -> str:
    external = [p for p in meta["packages"] if p["source"] is not None]
    members = sorted(p["name"] for p in meta["packages"] if p["source"] is None)
    usage = direct_usage(meta)

    lines: list[str] = []
    lines.append("# 依赖许可清单（机器生成，请勿手工编辑）")
    lines.append("")
    lines.append("> 由 `python3 scripts/gates/license_inventory.py` 从 `cargo metadata --locked` 生成。")
    lines.append("> CI 用 `--check` 对账：加依赖/升版本后忘记重新生成，这一项会变红。")
    lines.append("")
    lines.append(f"- `Cargo.lock` SHA-256（前 16 位）: `{lock_digest()}`")
    lines.append(f"- 外部依赖包数: **{len(external)}**（不含 {len(members)} 个 workspace 成员）")
    lines.append("- 许可来源: 各包 `Cargo.toml` 的 `license` 字段（SPDX 表达式，未经人工改写）")
    lines.append("")

    families: dict[str, int] = {}
    for pkg in external:
        key = pkg.get("license") or "(未声明)"
        families[key] = families.get(key, 0) + 1

    lines.append("## 许可族分布")
    lines.append("")
    lines.append("| SPDX 表达式 | 包数 |")
    lines.append("| :--- | ---: |")
    for key, count in sorted(families.items(), key=lambda kv: (-kv[1], kv[0])):
        lines.append(f"| `{key}` | {count} |")
    lines.append("")
    lines.append(
        "> 多许可表达式（`A OR B`）只要有一个分支在白名单内即通过 `cargo deny`；"
        "`AND` 则要求每一侧都被允许。`deny.toml` 的 `allow` 列表是唯一策略来源。"
    )
    lines.append("")

    lines.append("## 全量清单")
    lines.append("")
    lines.append("| 包 | 版本 | 许可 | 类型 | 直接依赖它的成员 |")
    lines.append("| :--- | :--- | :--- | :--- | :--- |")
    for pkg in sorted(external, key=lambda p: (p["name"].lower(), p["version"])):
        users = usage.get(pkg["name"])
        if users:
            kind = "直接"
            used_by = ", ".join(f"`{u}`" for u in sorted(users))
        else:
            kind = "传递"
            used_by = "—"
        license_expr = pkg.get("license") or "(未声明)"
        lines.append(f"| `{pkg['name']}` | `{pkg['version']}` | `{license_expr}` | {kind} | {used_by} |")
    lines.append("")
    lines.append("## 源码级移植（不由 cargo 依赖图覆盖）")
    lines.append("")
    lines.append(
        "从**历史项目源码**改写移植进来的代码不在 `cargo deny` 视野内，"
        "归属与许可全文必须由 `THIRD_PARTY_LICENSES.md` 承载。当前登记："
    )
    lines.append("")
    lines.append("- `synth-core`（MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors）—")
    lines.append("  `crates/yeban-dsp` 的部分模块；逐文件裁决见 `docs/ledger/dsp-core-provenance.md`。")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="依赖许可清单")
    parser.add_argument("--check", action="store_true", help="与已提交清单对账（不写文件）")
    args = parser.parse_args()

    rendered = render(metadata())

    if args.check:
        if not OUT.is_file():
            print(f"清单不存在: {OUT.relative_to(REPO)}（请先跑一次本脚本并提交）", file=sys.stderr)
            return 1
        current = OUT.read_text(encoding="utf-8")
        if current != rendered:
            print(
                f"依赖许可清单已漂移: {OUT.relative_to(REPO)} 与真实依赖图不一致。\n"
                "请运行 `python3 scripts/gates/license_inventory.py` 并提交结果。",
                file=sys.stderr,
            )
            return 1
        print(f"依赖许可清单与依赖图一致 ({len(rendered.splitlines())} 行)。")
        return 0

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(rendered, encoding="utf-8")
    print(f"已写入 {OUT.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
