#!/usr/bin/env python3
"""规范 ID 覆盖审计：**代码里声明的 ID** vs **四份规范里真实存在的 ID**。

它回答两个问题（两个方向都会出问题）：
1. **实现了但规范里没有** ⇒ 可能是**凭空发明的编号**。`AGENTS.md` §4.1 明确点名过一个先例：
   `MODEL-AST-006` 在规范里**缺号**，实现时不得凭空发明该编号。这一方向是**硬错误**。
2. **规范里有但没实现** ⇒ 正常（`ADR-0001 D7`：不具备条件的一律 PENDING），但要能**数得清**。

用法：
    python3 scripts/gates/spec_id_audit.py            # 人类可读报告
    python3 scripts/gates/spec_id_audit.py --check    # 只判方向 1（发明 ID）⇒ 有则退出码 1
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPECS = sorted(REPO.glob("docs/YEBAN_*.md"))
CRATES = REPO / "crates"

#: 规范 ID 的形态：`FAMILY-NNN`（全大写家族 + 三位数字），以及少量不带数字的单例（如 `MODEL-ISO-001` 有形）。
ID_RE = re.compile(r"\b([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*-\d{3})\b")
#: 代码里形如 `SPEC ID` 的字符串字面量（`IMPLEMENTED_SPEC_IDS` 数组的元素）。
DECL_RE = re.compile(r'"([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*-\d{3})"')


def spec_ids() -> set[str]:
    found: set[str] = set()
    for path in SPECS:
        found.update(ID_RE.findall(path.read_text(encoding="utf-8")))
    return found


def declared_ids() -> dict[str, set[str]]:
    """每个 crate 声明的 ID（只扫 `IMPLEMENTED_SPEC_IDS` 数组所在文件，避免把注释里的示例算进去）。"""
    per_crate: dict[str, set[str]] = {}
    for lib in sorted(CRATES.glob("*/src/lib.rs")):
        text = lib.read_text(encoding="utf-8")
        if "IMPLEMENTED_SPEC_IDS" not in text:
            continue
        start = text.index("IMPLEMENTED_SPEC_IDS")
        # ⚠ 必须从**赋值号之后**找方括号: 声明是 `pub const X: &[&str] = &[ … ]`,
        # 第一个 `[` 属于**类型** `&[&str]` —— 从那里扫描会一个字符串都取不到
        # （第一版就是这么错的: 报告"声明 0 个 ID", 而实际有三个 crate 声明了一堆）。
        assign = text.index("=", start)
        open_bracket = text.index("[", assign)
        depth = 0
        for index in range(open_bracket, len(text)):
            if text[index] == "[":
                depth += 1
            elif text[index] == "]":
                depth -= 1
                if depth == 0:
                    body = text[open_bracket:index]
                    break
        else:  # pragma: no cover - 语法异常时明确报错而不是静默
            raise SystemExit(f"{lib}: 找不到 IMPLEMENTED_SPEC_IDS 的结束方括号")
        per_crate[lib.parent.parent.name] = set(DECL_RE.findall(body))
    return per_crate


def main() -> int:
    check_only = "--check" in sys.argv
    in_spec = spec_ids()
    per_crate = declared_ids()
    declared: set[str] = set()
    for ids in per_crate.values():
        declared |= ids

    invented = sorted(declared - in_spec)
    missing = sorted(in_spec - declared)

    if invented:
        print("以下 ID 在**代码里声明**但**四份规范里不存在**（可能是凭空发明的编号）：", file=sys.stderr)
        for ident in invented:
            owners = sorted(name for name, ids in per_crate.items() if ident in ids)
            print(f"  - {ident}  （声明于 {', '.join(owners)}）", file=sys.stderr)
        print(
            "\n`AGENTS.md` §4.1 点名过先例（`MODEL-AST-006` 在规范里缺号）。"
            "处理方式：要么改代码用规范里真实的 ID，要么先在 ADR 里裁决并回写规范。",
            file=sys.stderr,
        )
        return 1

    if check_only:
        print(f"[ok] 规范 ID 审计（方向 1）: 代码声明的 {len(declared)} 个 ID 全部存在于四份规范中")
        return 0

    print("=== 规范 ID 覆盖审计 ===")
    print(f"四份规范里出现的 ID: {len(in_spec)}")
    print(f"代码里声明的 ID:     {len(declared)}（分布在 {len(per_crate)} 个 crate）")
    print(f"其中**未实现**:      {len(missing)}（正常 —— D7 的 PENDING 策略；列出前 20 个看分布）")
    for ident in missing[:20]:
        print(f"  - {ident}")
    if len(missing) > 20:
        print(f"  … 另 {len(missing) - 20} 个")
    print("\n每个 crate 声明数：")
    for name, ids in sorted(per_crate.items(), key=lambda item: -len(item[1])):
        print(f"  {name:22s} {len(ids)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
