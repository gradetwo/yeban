#!/usr/bin/env python3
"""s36：`schemas/project.schema.json` 里手抄的三份模型枚举名册，必须与模型**集合相等**。

数什么（单位：个）：
  ① 实现侧 = `crates/yeban-model/src/project.rs` 里三个 `pub enum` 的**变体名**集合
     （`TrackKind` / `DeviceKind` / `RoutingKind`）；
  ② 契约侧 = 同一份 schema 里三个**显式指针**指向的 `enum` 数组集合：
       properties.tracks.additionalProperties.properties.kind.enum
       properties.tracks.additionalProperties.properties.devices.items.properties.kind.enum
       properties.routing_graph.properties.edges.items.properties.kind.enum
判据：每个词表 ① == ②（**集合等式**，⛔ 只写下界会在"两边同时删空"时空绿），且两侧各自 ≥ 4。

为什么这三处值得单独一条守卫（现读，2026-11-01）：全仓搜 `"AuxReturn"` 只剩三处指同一份词汇 ——
模型枚举（真相）、`yeban-mcp` 侧的 `TRACK_KINDS`/`DEVICE_KINDS` 常量（已由 Rust 判据
`track_kinds_match_the_model_variant_names` 等钉回模型）、以及**本 schema 里这三处手抄**。
前两层是 Rust 编译期与测试面，schema 这一层没有任何机器对账 ⇒ 模型加一个 `TrackKind` 变体，
`.yeban` 归档校验会**静默拒绝**合法工程（`enum` 里少一项 ⇒ `validate_schemas.py` 照样绿）。

前提（本守卫同时守住，⛔ 只在成立时才可比）：三个枚举都**没有** `#[serde(rename…)]`／
`#[serde(rename_all…)]` ⇒ 变体名逐字等于 serde 名，文本抽取才是合法读法。
一旦出现 rename，抽取到的是 Rust 名而契约写的是 serde 名 ⇒ 本守卫**响亮报红并指向前提**，
⛔ 不许悄悄抽错（那时应改读运行期的 `serde_json::to_value(枚举)` 输出）。

⚠ 已知不覆盖（按 AGENTS.md §6.4 第 4 条登记，不冒充全知）：
  ① 只认**单元变体**（`Name,`）⇒ 若给这三个枚举加带字段的变体，本守卫报红而不是漏读；
  ② 只比**名字集合**，不比 schema 里 `kind` 的 `description` 措辞；
  ③ 三个指针是**写死的路径** ⇒ schema 重构（挪动 `kind` 的位置）会让守卫报"指针缺失"，
     这是有意的失败方式：宁可红，不可静默不数。
"""
import json
import re
import sys
from pathlib import Path

# 实现侧：模型源文件与三个枚举名。
MODEL_SRC = Path("crates/yeban-model/src/project.rs")
# 契约侧：枚举名 -> schema 里的显式指针（点号分隔的键路径）。
SITES = {
    "TrackKind": "properties.tracks.additionalProperties.properties.kind.enum",
    "DeviceKind": "properties.tracks.additionalProperties.properties.devices.items.properties.kind.enum",
    "RoutingKind": "properties.routing_graph.properties.edges.items.properties.kind.enum",
}
LOWER_BOUND = 4

ENUM_HEAD = re.compile(r"^pub enum (\w+) \{\s*$")
VARIANT = re.compile(r"^    ([A-Z][A-Za-z0-9]*),\s*$")
NON_UNIT = re.compile(r"^    ([A-Z][A-Za-z0-9]*)\s*[{(]")
SERDE_ATTR = re.compile(r"#\[serde\((.*)\)\]")


def fail(msg: str) -> None:
    print(f"[FAIL] {msg}")
    sys.exit(1)


def block_lines(lines: list[str], start: int) -> list[str]:
    """取 `pub enum` 的**属性块**：紧邻其上、由文档注释／属性组成的连续行。"""
    block = []
    for i in range(start - 1, -1, -1):
        stripped = lines[i].strip()
        if stripped.startswith("///") or stripped.startswith("#[") or stripped == "":
            block.insert(0, stripped)
            if stripped == "" and block[-1] != "":
                # 空行是块的边界：属性块与上一条声明之间总隔着空行。
                pass
            continue
        break
    return block


def serde_rename_in(block: list[str]) -> list[str]:
    """属性块里出现的 `serde(...)` 参数中含 `rename` 的项（前提检查）。"""
    hits = []
    for line in block:
        m = SERDE_ATTR.search(line)
        if m and "rename" in m.group(1):
            hits.append(line.strip())
    return hits


def extract_variants(text: str, enum_name: str) -> list[str]:
    """抽 `pub enum <enum_name>` 的单元变体名（按声明顺序）。"""
    lines = text.splitlines()
    for idx, line in enumerate(lines):
        m = ENUM_HEAD.match(line)
        if not m or m.group(1) != enum_name:
            continue
        # 前提：紧邻属性块里不得出现 serde rename。
        renames = serde_rename_in(block_lines(lines, idx))
        if renames:
            fail(
                f"`{enum_name}` 出现了 serde rename ⇒ 本守卫"
                "「变体名 == serde 名」的文本抽取前提失效，请改读运行期序列化输出后再比："
                f"{'; '.join(renames)}"
            )
        variants = []
        for body in lines[idx + 1 :]:
            if body == "}":
                break
            nm = NON_UNIT.match(body)
            if nm:
                fail(
                    f"`{enum_name}` 的变体 `{nm.group(1)}` 带字段（非单元变体）"
                    "⇒ 本守卫只读单元变体，请改判据形态，⛔ 别让它静默少读一项"
                )
            vm = VARIANT.match(body)
            if vm:
                variants.append(vm.group(1))
        if not variants:
            fail(f"`{enum_name}` 抽到空变体集 ⇒ 抽取器或源文件形态变了，本判据空转")
        return variants
    fail(f"`{MODEL_SRC}` 里找不到 `pub enum {enum_name}`（模型重构了？指针要跟着改）")
    return []  # 不可达，供类型检查


def resolve(schema: dict, pointer: str) -> list:
    node = schema
    for key in pointer.split("."):
        if not isinstance(node, dict) or key not in node:
            fail(f"schema 指针 `{pointer}` 在 `.{key}` 处断裂 ⇒ 契约侧名册挪窝了，请同步本守卫")
        node = node[key]
    if not isinstance(node, list):
        fail(f"schema 指针 `{pointer}` 的终点不是数组，而是 {type(node).__name__}")
    return node


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    if "--root" in sys.argv:
        root = Path(sys.argv[sys.argv.index("--root") + 1]).resolve()

    src = root / MODEL_SRC
    if not src.is_file():
        fail(f"实现侧文件不存在: {src}")
    schema_path = root / "schemas" / "project.schema.json"
    if not schema_path.is_file():
        fail(f"契约文件不存在: {schema_path}")
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    text = src.read_text(encoding="utf-8")

    total = 0
    for enum_name, pointer in sorted(SITES.items()):
        impl = extract_variants(text, enum_name)
        contract = resolve(schema, pointer)
        impl_set, contract_set = set(impl), set(contract)
        print(
            f"[读数] `{enum_name}`：实现侧 {len(impl_set)} 个（{', '.join(sorted(impl_set))}）"
            f"／契约侧 {len(contract_set)} 个（{', '.join(sorted(contract_set))}）"
        )
        # ⚠ 顺序是**读数顺序**的一部分：集合差要先于下界。
        # 先查下界会让"契约少一项（3/4）"报成"被掏空了？"—— 把**少了哪个名字**这条最有用的信息
        # 埋在一条粗报警后面。下界只在该查"两边同时删空"（差集为空、等式空转）时才不可替代。
        dup = sorted({c for c in contract if contract.count(c) > 1})
        if dup:
            fail(f"`{enum_name}` 的契约 enum 有重复条目: {', '.join(dup)}")
        missing = sorted(impl_set - contract_set)
        if missing:
            fail(
                f"`{enum_name}`：模型里有、schema 里没登记 ⇒ `.yeban` 归档会**静默拒绝**合法工程: "
                f"{', '.join(missing)}"
            )
        extra = sorted(contract_set - impl_set)
        if extra:
            fail(f"`{enum_name}`：schema 里登记了、模型里没有（幽灵条款）: {', '.join(extra)}")
        if len(impl_set) < LOWER_BOUND:
            fail(f"`{enum_name}` 实现侧只剩 {len(impl_set)} 个（下界 {LOWER_BOUND}）—— 模型枚举被掏空了？")
        if len(contract_set) < LOWER_BOUND:
            fail(
                f"`{enum_name}` 契约侧只剩 {len(contract_set)} 个（下界 {LOWER_BOUND}）"
                f"，指针 `{pointer}` —— 两边同名但被成对删空，等式空转 ⇒ 看下界"
            )
        total += len(impl_set)

    print(
        f"[ok] 三份模型枚举名册的实现与契约**集合相等**（合计 {total} 个名字，"
        f"每份 ≥ 下界 {LOWER_BOUND}）⇒ `schemas/project.schema.json` 不再是不对账的手抄"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
