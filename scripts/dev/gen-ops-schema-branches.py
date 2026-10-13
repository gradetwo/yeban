#!/usr/bin/env python3
"""从模型枚举重新生成 `schemas/ops.schema.json` 的 `properties.op.oneOf`。

为什么有它（提案 4）：每加一个 `Op` 变体，集成者都要**手写**一个 schema 分支，
而所有分支的**形状完全相同**（`{required:[Name], properties:{Name:{type:object}}}`）。
手写带来一次**两个来回**的 SCHEMA_REQUEST 流程（线报 ⇒ 我改 ⇒ 推 ⇒ 线变基再跑）。

用法：
    scripts/dev/gen-ops-schema-branches.py            # 就地重写
    scripts/dev/gen-ops-schema-branches.py --check    # 只报差异，⛔ 不写（CI 可用）

⚠ 变体清单**从源码现读**，⛔ 不手抄（台账第 44 条：手抄清单会漂移）。
⚠ 抽不到东西 ⇒ **响亮失败**（台账第 41 条），⛔ 不许静默写一个空 oneOf。
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OPS_RS = ROOT / "crates/yeban-model/src/ops.rs"
SCHEMA = ROOT / "schemas/ops.schema.json"


def variant_names(source: str) -> list[str]:
    """按声明顺序取 `pub enum Op` 的顶层变体名。"""
    start = source.find("pub enum Op {")
    if start < 0:
        raise SystemExit("ops.rs: 找不到 `pub enum Op {`（抽取规则失效 ⇒ 响亮失败）")

    # 扫到**行首 `}`** 为止。⛔ 不按花括号配对：文档注释里有 `Op::X { .. }` 示例。
    body = []
    for line in source[start:].splitlines()[1:]:
        if line == "}":
            break
        body.append(line)

    names = []
    for line in body:
        if not line.startswith("    ") or line.startswith("     "):
            continue  # 只取**恰好四个空格**缩进的行 ⇒ 顶层变体
        rest = line[4:]
        m = re.match(r"([A-Z][A-Za-z0-9]*)", rest)
        if not m:
            continue
        tail = rest[m.end():].lstrip()
        if tail.startswith("{") or tail.startswith(","):
            names.append(m.group(1))

    if len(names) < 10:
        raise SystemExit(
            f"ops.rs: 只抽到 {len(names)} 个变体 ⇒ 抽取规则失效（响亮失败，⛔ 不写文件）"
        )
    if len(set(names)) != len(names):
        raise SystemExit("ops.rs: 抽到重复变体名 ⇒ 抽取规则失效")
    return names


def main() -> int:
    check_only = "--check" in sys.argv[1:]
    names = variant_names(OPS_RS.read_text(encoding="utf-8"))

    with SCHEMA.open(encoding="utf-8") as fh:
        doc = json.load(fh, object_pairs_hook=dict)

    branches = [
        {"required": [n], "properties": {n: {"type": "object"}}} for n in names
    ]
    doc["properties"]["op"]["oneOf"] = branches

    rendered = json.dumps(doc, indent=2, ensure_ascii=False) + "\n"
    current = SCHEMA.read_text(encoding="utf-8")

    if rendered == current:
        print(f"gen-ops-schema-branches: 已是最新（{len(names)} 个分支）")
        return 0

    if check_only:
        old = json.loads(current)["properties"]["op"]["oneOf"]
        old_names = [b["required"][0] for b in old]
        print(f"gen-ops-schema-branches: ⛔ 契约与枚举不一致")
        print(f"  枚举 {len(names)} 个: {names}")
        print(f"  契约 {len(old_names)} 个: {old_names}")
        print(f"  缺（枚举有、契约无）: {[n for n in names if n not in old_names]}")
        print(f"  多（契约有、枚举无）: {[n for n in old_names if n not in names]}")
        return 1

    SCHEMA.write_text(rendered, encoding="utf-8")
    print(f"gen-ops-schema-branches: 已重写（{len(names)} 个分支）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
