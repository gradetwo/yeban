#!/usr/bin/env python3
"""文档链接与 README 双语契约检查 (docs link-rot gate)。

为什么值得一条门禁：

这个仓库的主要产物是**文档**（Normative 规范（2026-10-06 起桌面 UI/UX 规范已降级为建议与参考） + 操作手册 + 账本），而 Agent 与人类都靠链接
在文档之间跳转。链接腐烂不会让任何测试变红，却会让"权威事实源"变成死路 —— 这正是 SKILL 说的
"文档漂移要机械化拦住，而不是靠自觉"。

两种严重度（因为红线 1 的现实约束）：

- **错误（让 CI 变红）**：相对路径链接指向不存在的文件；或**非**法务文件里出现 `file://` 绝对路径。
- **警告（只报告，不变红）**：`LICENSE` / `LEGAL.md` / `SECURITY.md` / `TRADEMARK.md` /
  `GOVERNANCE.md` / `NOTICE.md` 里的 `file://` 链接。AGENTS.md §2 红线 1 明令 Agent 不得修改这些文件，
  所以这里**不能**用一条永远红（或被迫关掉）的判据去逼 Agent 违规。它被报告出来，等人类负责人处理。

额外断言（README 双语契约，来自用户要求"中英文两份、默认英文、logo 在顶部"）：
  R1 `README.md` 与 `README.zh-CN.md` 必须同时存在
  R2 两份互相链接（语言切换）
  R3 两份都在顶部带品牌 logo（`<picture>` + 明暗两套源）
  R4 `README.md` 必须包含官网与联系邮箱

用法:
    python3 scripts/gates/check_docs_links.py
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

SKIP_DIRS = {".git", "target", "node_modules", ".worktrees", "dist", ".cargo-home", ".cache", "artifacts"}


def rel_parts(path: Path) -> tuple[str, ...]:
    """相对**仓库根**的路径分量。

    为什么不能直接用 `path.parts`: 工作线是在 `<main>/.worktrees/<line>/` 里跑的, 于是
    绝对路径的每一段都含 `.worktrees` ⇒ 若用绝对分量做跳过判定, **工作线里所有文件都会被跳过**,
    本机门禁静默变成空跑（实测"扫描 0 个 markdown"）。
    这正是"本机绿、CI 红"的来源: engine-rt 第 1 轮的文档红点就是这么漏掉的。
    """
    try:
        return path.resolve().relative_to(REPO.resolve()).parts
    except ValueError:
        return path.parts

#: 红线 1 保护的文件：其内的 file:// 链接只报告，不阻断（Agent 不得修改它们）
PROTECTED_LEGAL = {
    "LICENSE",
    "LEGAL.md",
    "SECURITY.md",
    "TRADEMARK.md",
    "GOVERNANCE.md",
    "NOTICE.md",
}

FENCE_RE = re.compile(r"^\s*(```|~~~)")
LINK_RE = re.compile(r"\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")


def markdown_files() -> list[Path]:
    out: list[Path] = []
    for path in REPO.rglob("*.md"):
        if any(part in SKIP_DIRS for part in rel_parts(path)):
            continue
        out.append(path)
    return sorted(out)


def strip_fences(text: str) -> str:
    """去掉围栏代码块，避免把示例里的链接当成真链接。"""
    keep: list[str] = []
    inside = False
    for line in text.splitlines():
        if FENCE_RE.match(line):
            inside = not inside
            keep.append("")
            continue
        keep.append("" if inside else line)
    return "\n".join(keep)


def check_links() -> tuple[list[str], list[str], int]:
    errors: list[str] = []
    warnings: list[str] = []
    checked = 0

    for path in markdown_files():
        rel_file = path.relative_to(REPO)
        body = strip_fences(path.read_text(encoding="utf-8", errors="replace"))
        for match in LINK_RE.finditer(body):
            target = match.group(1)
            if target.startswith(("http://", "https://", "mailto:", "#", "data:")):
                continue
            checked += 1
            line_no = body[: match.start()].count("\n") + 1

            if target.startswith("file://"):
                msg = f"{rel_file}:{line_no} 使用 file:// 绝对路径: {target}"
                if str(rel_file) in PROTECTED_LEGAL:
                    warnings.append(msg + "  (红线 1 保护文件，需人类负责人修复)")
                else:
                    errors.append(msg)
                continue

            clean = target.split("#")[0].split("?")[0]
            if not clean:
                continue
            resolved = (path.parent / clean).resolve()
            if not resolved.exists():
                errors.append(f"{rel_file}:{line_no} 链接指向不存在的路径: {target}")

    return errors, warnings, checked


def check_readme_pair() -> list[str]:
    problems: list[str] = []
    en = REPO / "README.md"
    zh = REPO / "README.zh-CN.md"

    if not en.is_file():
        problems.append("R1 缺少 README.md（GitHub 默认展示英文版）")
    if not zh.is_file():
        problems.append("R1 缺少 README.zh-CN.md（中文版）")
    if problems:
        return problems

    en_text = en.read_text(encoding="utf-8")
    zh_text = zh.read_text(encoding="utf-8")

    if "README.zh-CN.md" not in en_text:
        problems.append("R2 README.md 没有链接到 README.zh-CN.md")
    if "README.md" not in zh_text:
        problems.append("R2 README.zh-CN.md 没有链接回 README.md")

    for name, text in (("README.md", en_text), ("README.zh-CN.md", zh_text)):
        if "<picture>" not in text:
            problems.append(f"R3 {name} 顶部缺少 <picture> 品牌 logo")
        for variant in ("yeban-dark-256.png", "yeban-light-256.png"):
            if variant not in text:
                problems.append(f"R3 {name} 缺少 {variant}（明暗两套 logo 源）")
        if not text.lstrip().startswith("<p align=\"center\">"):
            problems.append(f"R3 {name} 的 logo 不在文件顶部")

    for needle, label in (
        ("https://yeban.wangda.today", "官网地址"),
        ("yeban@wangda.today", "联系邮箱"),
    ):
        if needle not in en_text:
            problems.append(f"R4 README.md 缺少{label} {needle}")

    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description="文档链接与 README 双语契约检查")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()

    errors, warnings, checked = check_links()
    errors.extend(check_readme_pair())

    files = len(markdown_files())
    if not args.quiet:
        for w in warnings:
            print(f"[warn] {w}")
        print(f"note: 扫描 {files} 个 markdown 文件，检查 {checked} 个相对链接")

    if errors:
        print(f"\n文档检查未通过 ({len(errors)} 项):", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        if warnings:
            print(f"\n另有 {len(warnings)} 条 file:// 警告（保护文件，需人类处理）", file=sys.stderr)
        return 1

    extra = f"，{len(warnings)} 条保护文件警告（不阻断）" if warnings else ""
    print(f"文档检查通过（{files} 个文件{extra}）。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
