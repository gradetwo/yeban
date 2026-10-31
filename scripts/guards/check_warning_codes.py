#!/usr/bin/env python3
"""s28：`warnings[].code` 的名册必须是**一份**真相 —— 契约（schema）与实现（Rust 构造点）**集合相等**。

数什么（单位：个）：
  ① 实现侧 = `crates/yeban-mcp/src/` 里 `"code": "camelCase"` 形态的**不同取值**个数；
  ② 契约侧 = `schemas/mcp-tools.schema.json` 里 `definitions.WarningCode.enum` 的**不同条目**个数。
判据：① == ②（**集合等式**，⛔ 只写下界会在"两边同时删空"时空绿），且 ① ≥ 下界 5。

⚠ 已知不覆盖（按纪律登记，不冒充全知）：只认**字面量**形态。将来若出现变量拼接的 code，
本守卫会漏 —— 与台账 #44（"空集即期望值"不可机械化）、§0.6.0p 候选 B（"有 fn 即算过"）同一类盲点。
"""
import json
import re
import sys
from pathlib import Path

CAMEL = re.compile(r"^[a-z]+[A-Za-z0-9]*$")
SITE = re.compile(r'"code"\s*:\s*"([A-Za-z][A-Za-z0-9]*)"')
LOWER_BOUND = 5


def fail(msg: str) -> None:
    print(f"[FAIL] {msg}")
    sys.exit(1)


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    if "--root" in sys.argv:
        root = Path(sys.argv[sys.argv.index("--root") + 1]).resolve()

    src = root / "crates" / "yeban-mcp" / "src"
    if not src.is_dir():
        fail(f"实现侧目录不存在: {src}")

    impl = {}
    for path in sorted(src.rglob("*.rs")):
        for m in SITE.finditer(path.read_text(encoding="utf-8")):
            code = m.group(1)
            # 只登记 camelCase 形态；错误码（NOT_IMPLEMENTED 等全大写带下划线）不是 warnings 的词汇表。
            if CAMEL.match(code):
                impl.setdefault(code, str(path.relative_to(root)))

    schema_path = root / "schemas" / "mcp-tools.schema.json"
    if not schema_path.is_file():
        fail(f"契约文件不存在: {schema_path}")
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    defs = schema.get("definitions", {})
    wc = defs.get("WarningCode")
    if wc is None:
        fail("schemas/mcp-tools.schema.json 缺 definitions.WarningCode —— `warnings[].code` 的名册没有落进契约")
    enum = wc.get("enum")
    if not isinstance(enum, list):
        fail("definitions.WarningCode.enum 不是数组")

    contract = set(enum)
    impl_set = set(impl)

    print(f"[读数] 实现侧 code {len(impl_set)} 个（构造点逐名: {', '.join(sorted(impl))}）")
    print(f"[读数] 契约侧 enum {len(contract)} 个: {', '.join(sorted(contract))}")

    if len(impl_set) < LOWER_BOUND:
        fail(f"实现侧只剩 {len(impl_set)} 个 code（下界 {LOWER_BOUND}）—— 构造点被掏空了？")
    missing = impl_set - contract
    if missing:
        fail(f"实现里有、契约里没登记（契约漏广播）: {', '.join(sorted(missing))}")
    extra = contract - impl_set
    if extra:
        fail(f"契约里登记了、实现里没有（幽灵条款）: {', '.join(sorted(extra))}")
    dup = [c for c in enum if enum.count(c) > 1]
    if dup:
        fail(f"enum 有重复条目: {', '.join(sorted(set(dup)))}")
    bad = [c for c in enum if not CAMEL.match(c)]
    if bad:
        fail(f"enum 有条目不是 camelCase 形态: {', '.join(sorted(bad))}")

    print("[ok] `warnings[].code` 的实现与契约**集合相等**，名册只有一份真相（schema）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
