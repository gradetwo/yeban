#!/usr/bin/env python3
"""s36/s38：`schemas/project.schema.json` 里每一处手抄的模型名册，必须与模型**集合相等**。

数什么（单位：**处** = 一个 schema `enum` 数组 ↔ 一个模型侧生产者）：
  `schemas/project.schema.json` 全文现读共 **14** 处 `enum` 数组，本守卫给每一处登记
  一条「模型侧怎么抽出同一份词汇」的规格，逐处判 **① 实现侧 == ② 契约侧**（集合等式），
  并要求每一侧各自 ≥ 该处登记的**下界**。
  ⛔ 只写下界会在「两边同时删一格」时空绿 ⇒ 等式是主判据，下界只补"成对删空"那一刀。

三类生产者（同一份词汇在模型里有三种写法，抽取器必须三种都会读）：
  **A `variants`** — 派生 serde 的 `pub enum`：变体名逐字 == JSON 名（前提：无 `rename`）。
      覆盖 **10** 处（`TrackKind`/`DeviceKind`/`RoutingKind`/`BitDepth`/`PanLaw`/
      `LaunchQuantization`/`AutomationWriteMode`/`MediaKind`，以及同一个 `CurveType` 的**两个**站点）。
  **B `int_serde`** — **手写** `Serialize`/`Deserialize` 的整数枚举（`SampleRate`/`BlockSize`）：
      文档注释明写「契约要求整数，派生枚举会写成字符串」⇒ 变体名（`Hz44100`）与 JSON 值（44100）
      **不同轨**，A 类读法在这里必然报错。正解是读那两张真表：`pub const ALL: [Self; N]`
      给出成员，`Self::<变体> => <整数>` 的 match 臂给出数值 ⇒ 守卫比的是**数值集合**。
  **C `matches`** — 没有枚举、只有 `matches!(self.<字段>, 1 | 2 | 4 | …)` 白名单（拍号分母两处）：
      直接读那条 `matches!`，⇒ 词汇仍只有一个生产者，⛔ 第二份手抄。

为什么值得单独一条守卫（现读，2026-11-01）：全仓搜 `"AuxReturn"` 只剩三处指同一份词汇 ——
模型枚举（真相）、`yeban-mcp` 侧的 `TRACK_KINDS`/`DEVICE_KINDS` 常量（已由 Rust 判据
`track_kinds_and_device_kinds_match_the_model` 一类钉回模型）、以及**本 schema 里的手抄**。
前两层在 Rust 编译期与测试面，schema 这一层原先没有任何机器对账 ⇒ 模型加一个变体，
`.yeban` 归档校验会**静默拒绝**合法工程（`enum` 里少一项 ⇒ `validate_schemas.py` 照样绿）。

前提（本守卫同时守住，⛔ 只在成立时才可比）：A 类枚举**不得**带 `#[serde(rename…)]`／
`#[serde(rename_all…)]` —— 枚举级与**变体级**都查（变体级的 `#[serde(rename)]` 只改那一格，
比枚举级更难从契约里看出来，所以同样判前提失效）。一旦出现，文本抽取到的是 Rust 名而契约写的是
serde 名 ⇒ 本守卫**响亮报红并指向前提**，⛔ 不许悄悄抽错（那时应改读运行期 `serde_json::to_value`）。

⚠ 已知不覆盖（按 AGENTS.md §6.4 第 4 条登记，不冒充全知）：
  ① 只管 `schemas/project.schema.json` ⇒ `schemas/mcp-tools.schema.json` 的 enum 由
     `check_warning_codes.py`（warnings 名册）与 Rust 侧判据承担，不在本文件；
  ② A 类只认**单元变体**（`Name,`）⇒ 给这些枚举加带字段的变体，本守卫报红而不是漏读；
  ③ 只比**名字/数值集合**，不比 schema 里的 `description` 措辞；
  ④ 指针是**写死的** ⇒ schema 重构（挪动位置）会报"指针断裂"，这是有意的失败方式：宁可红，不可静默不数；
  ⑤ 下界是**手登记的整数**：它的唯一职责是堵"成对删空"，所以只在**删格**时需要人工上调，
     加格不需要（加格让两侧一起变大，等式照成立）。⚠ 别把它读成"名册应有 N 项"的规范值。
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

#: 一处站点 = (读数用的名字, 生产者类型, 模型侧源文件, 抽取参数, schema 指针, 下界)。
#: 下界 = 该处**当前**的实现侧条目数（见模块文档 ⚠⑤：删格才需要动它）。
SITES: list[tuple[str, str, str, tuple[str, ...], str, int]] = [
    # --- A 类：派生 serde 的枚举（变体名 == JSON 名）---
    (
        "TrackKind",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("TrackKind",),
        "properties.tracks.additionalProperties.properties.kind.enum",
        4,
    ),
    (
        "DeviceKind",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("DeviceKind",),
        "properties.tracks.additionalProperties.properties.devices.items.properties.kind.enum",
        4,
    ),
    (
        "RoutingKind",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("RoutingKind",),
        "properties.routing_graph.properties.edges.items.properties.kind.enum",
        4,
    ),
    (
        "BitDepth",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("BitDepth",),
        "properties.audio_config.properties.bit_depth.enum",
        3,
    ),
    (
        "PanLaw",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("PanLaw",),
        "properties.audio_config.properties.pan_law.enum",
        4,
    ),
    (
        "LaunchQuantization",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("LaunchQuantization",),
        "properties.transport.properties.launch_quantization.enum",
        5,
    ),
    (
        "AutomationWriteMode",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("AutomationWriteMode",),
        "properties.tracks.additionalProperties.properties.automation_lanes.items"
        ".properties.write_mode.enum",
        4,
    ),
    (
        "MediaKind",
        "variants",
        "crates/yeban-model/src/project.rs",
        ("MediaKind",),
        "properties.assets.additionalProperties.properties.media_kind.enum",
        3,
    ),
    # ⭐ `CurveType` 住在 `music.rs`（⛔ 不在 project.rs）⇒ 源文件必须是**每站点一条**，
    # 否则 s38 加不进这一处，只能把它登记成"不覆盖"。
    (
        "CurveType(automation 点)",
        "variants",
        "crates/yeban-model/src/music.rs",
        ("CurveType",),
        "properties.tracks.additionalProperties.properties.automation_lanes.items"
        ".properties.points.additionalProperties.properties.curve.enum",
        4,
    ),
    (
        "CurveType(片段滑音)",
        "variants",
        "crates/yeban-model/src/music.rs",
        ("CurveType",),
        # ⚠ 这条指针**穿过数组下标**（`oneOf[0]`／`oneOf[1]`）⇒ 解析器要认 `[N]`。
        "properties.clip_pool.additionalProperties.properties.content"
        ".oneOf[0].properties.Midi.properties.notes.additionalProperties"
        ".properties.slide.oneOf[1].properties.curve.enum",
        4,
    ),
    # --- B 类：手写 serde 的整数枚举（比的是**数值**，不是名字）---
    (
        "SampleRate",
        "int_serde",
        "crates/yeban-model/src/project.rs",
        ("SampleRate", "ALL", "hz"),
        "properties.audio_config.properties.sample_rate.enum",
        5,
    ),
    (
        "BlockSize",
        "int_serde",
        "crates/yeban-model/src/project.rs",
        ("BlockSize", "ALL", "frames"),
        "properties.audio_config.properties.block_size.enum",
        5,
    ),
    # --- C 类：没有枚举，只有 `matches!` 白名单（同一词汇的两个站点）---
    (
        "TimeSignature.denominator",
        "matches",
        "crates/yeban-model/src/project.rs",
        ("denominator",),
        "properties.time_signature.properties.denominator.enum",
        6,
    ),
    (
        "TimeSignature.denominator(地图)",
        "matches",
        "crates/yeban-model/src/project.rs",
        ("denominator",),
        "properties.time_signature_map.additionalProperties.properties.denominator.enum",
        6,
    ),
]

ENUM_HEAD = re.compile(r"^pub enum (\w+) \{\s*$")
#: ⚠ 字符集含下划线：`PanLaw::ConstantPowerMinus4_5dB` 实测被 s36 的第一版**漏读**过
#: （`[A-Za-z0-9]*` 不认 `_` ⇒ 实现侧少一格 ⇒ 报成"schema 登记了、模型里没有"的**假红**，
#: 而真相是抽取器读不动这个名字。s38 现读 `PanLaw` 四处里就有一处带 `_`。）
VARIANT = re.compile(r"^    ([A-Z][A-Za-z0-9_]*),\s*$")
NON_UNIT = re.compile(r"^    ([A-Z][A-Za-z0-9_]*)\s*[{(]")
SERDE_ATTR = re.compile(r"#\[serde\((.*)\)\]")
#: `pub const ALL: [Self; 5] = [Self::Hz44100, Self::Hz48000, …];`
ALL_ARRAY = "pub const ALL: [Self;"
#: `Self::Hz44100 => 44_100,`（accessor 的 match 臂 —— 数值在这里，⛔ 不在 schema 里）
ARM = re.compile(r"Self::(\w+)\s*=>\s*([0-9][0-9_]*)\s*,")
#: `matches!(self.denominator, 1 | 2 | 4 | 8 | 16 | 32)`
MATCHES = re.compile(r"matches!\s*\(\s*self\.(\w+)\s*,\s*([^)]+?)\s*\)")


def fail(msg: str) -> None:
    print(f"[FAIL] {msg}")
    sys.exit(1)


# ---------------------------------------------------------------------------
# 契约侧：点号路径 ＋ 可选的 `[N]` 下标
# ---------------------------------------------------------------------------

def resolve(schema, pointer: str):
    node = schema
    for step in pointer.split("."):
        m = re.fullmatch(r"(\w+)\[(\d+)\]", step)
        if m:
            key, index = m.group(1), int(m.group(2))
            if not isinstance(node, dict) or key not in node:
                fail(f"schema 指针 `{pointer}` 在 `.{key}` 处断裂 ⇒ 契约侧名册挪窝了，请同步本守卫")
            node = node[key]
            if not isinstance(node, list) or index >= len(node):
                fail(f"schema 指针 `{pointer}` 的 `.{key}[{index}]` 越界或非数组 ⇒ 契约侧改形了，请同步本守卫")
            node = node[index]
        else:
            if not isinstance(node, dict) or step not in node:
                fail(f"schema 指针 `{pointer}` 在 `.{step}` 处断裂 ⇒ 契约侧名册挪窝了，请同步本守卫")
            node = node[step]
    if not isinstance(node, list):
        fail(f"schema 指针 `{pointer}` 的终点不是数组，而是 {type(node).__name__}")
    return node


# ---------------------------------------------------------------------------
# A 类：枚举变体名
# ---------------------------------------------------------------------------

def attribute_block(lines: list[str], start: int) -> list[str]:
    """取 `pub enum` 的**属性块**：紧邻其上、由文档注释／属性组成的连续行。"""
    block: list[str] = []
    for i in range(start - 1, -1, -1):
        stripped = lines[i].strip()
        if stripped.startswith("///") or stripped.startswith("#[") or stripped == "":
            block.insert(0, stripped)
            continue
        break
    return block


def serde_renames(lines: list[str], where: str) -> None:
    """前提检查：出现 serde rename 就报红并指向前提（⛔ 不许悄悄抽错）。"""
    hits = [line for line in lines if (m := SERDE_ATTR.search(line)) and "rename" in m.group(1)]
    if hits:
        fail(
            f"`{where}` 出现了 serde rename ⇒ 本守卫「变体名 == serde 名」的文本抽取前提失效，"
            f"请改读运行期序列化输出后再比：{'; '.join(hits)}"
        )


def enum_body(text: str, enum_name: str) -> tuple[list[str], str]:
    """返回 (单元变体名列表, 枚举体原文)；前提（rename）与形态（非单元变体）都在此守住。"""
    lines = text.splitlines()
    for idx, line in enumerate(lines):
        m = ENUM_HEAD.match(line)
        if not m or m.group(1) != enum_name:
            continue
        serde_renames(attribute_block(lines, idx), f"`{enum_name}` 的枚举级属性")
        body: list[str] = []
        for row in lines[idx + 1 :]:
            if row == "}":
                break
            serde_renames([row], f"`{enum_name}` 的变体级属性")
            if NON_UNIT.match(row):
                nm = NON_UNIT.match(row).group(1)  # type: ignore[union-attr]
                fail(
                    f"`{enum_name}` 的变体 `{nm}` 带字段（非单元变体）"
                    "⇒ 本守卫只读单元变体，请改判据形态，⛔ 别让它静默少读一项"
                )
            vm = VARIANT.match(row)
            if vm:
                body.append(vm.group(1))
        if not body:
            fail(f"`{enum_name}` 抽到空变体集 ⇒ 抽取器或源文件形态变了，本判据空转")
        return body, "\n".join(lines[idx + 1 :])
    fail(f"源文件里找不到 `pub enum {enum_name}`（模型重构了？指针要跟着改）")
    return [], ""  # 不可达，供类型检查


# ---------------------------------------------------------------------------
# B 类：手写 serde 的整数枚举 —— 读 `ALL` 与 accessor 的 match 臂
# ---------------------------------------------------------------------------

def int_vocabulary(text: str, enum_name: str, all_name: str, accessor: str) -> list[int]:
    """从 `pub const ALL` 的成员名 + accessor 的 `Self::X => N` 臂导出**数值**名册。

    ⚠ 两处查找都必须**从 `pub enum <enum_name>` 那一行往后**开始（s38 实测的第一版从文件头找，
    于是 `BlockSize` 拿到的是 `SampleRate` 的 `ALL` 与 `hz()` ⇒ 报「成员在 frames() 里没有数值臂」。
    同一份文件里两个同构枚举是这里的常态，⛔ 全局 `find` 读的是"第一个"而不是"这一个"）。
    """
    head = re.search(r"^pub enum %s \{\s*$" % re.escape(enum_name), text, re.M)
    if not head:
        fail(f"源文件里找不到 `pub enum {enum_name}`（模型重构了？指针要跟着改）")
    after = head.end()

    marker = f"{ALL_ARRAY} "
    start = text.find(marker, after)
    if start < 0:
        fail(f"`{enum_name}` 里读不到 `{marker.strip()}` ⇒ 手写 serde 的整数名册没了唯一生产者，请同步本守卫")
    head_idx = text.index("[", text.index("=", start))
    end = text.index("]", head_idx)
    members = re.findall(r"Self::(\w+)", text[head_idx:end])
    if not members:
        fail(f"`{enum_name}.{all_name}` 抽到空成员列表 ⇒ 抽取器失效")

    # accessor 的 match 臂：只在**该 accessor 的函数体**里找，⛔ 全文件找（别处也可能有 Self::X => 整数）。
    fn = re.search(r"fn %s\(self\)[^{{]*\{" % re.escape(accessor), text[after:])
    if not fn:
        fail(
            f"`{enum_name}` 里读不到 accessor `fn {accessor}(self)` ⇒ "
            "手写 serde 靠它把变体映成整数，名字改了就要同步本守卫"
        )
    body = text[after:][fn.end() : text[after:].index("\n    }", fn.end())]
    arms = ARM.findall(body)
    if not arms:
        fail(f"`{enum_name}::{accessor}()` 的函数体里读不到任何 `Self::X => 整数` 臂 ⇒ 抽取器失效")
    table = {}
    for name, literal in arms:
        value = int(literal.replace("_", ""))
        if name in table and table[name] != value:
            fail(f"`{enum_name}::{accessor}()` 给 `{name}` 两条不同的数值臂（{table[name]} / {value}）")
        table[name] = value
    missing = [m for m in members if m not in table]
    if missing:
        fail(
            f"`{enum_name}.{all_name}` 的成员 {', '.join(missing)} 在 `{accessor}()` 里没有数值臂"
            "⇒ 序列化会走不到，本守卫无法比数值名册"
        )
    return [table[m] for m in members]


# ---------------------------------------------------------------------------
# C 类：`matches!(self.<字段>, 1 | 2 | …)` 白名单
# ---------------------------------------------------------------------------

def matches_vocabulary(text: str, field: str) -> list[int]:
    hits = [m for m in MATCHES.finditer(text) if m.group(1) == field]
    if not hits:
        fail(f"源文件里读不到 `matches!(self.{field}, …)` ⇒ 该词汇的唯一生产者换了写法，请同步本守卫")
    if len(hits) > 1:
        sets = [tuple(sorted(int(v.strip()) for v in h.group(2).split("|"))) for h in hits]
        if len(set(sets)) > 1:
            fail(f"源文件里 `{field}` 有**多份且不同**的 matches! 白名单: {sets} ⇒ 模型侧自己就没对上")
        return list(sets[0])
    values = []
    for token in hits[0].group(2).split("|"):
        token = token.strip()
        m = re.fullmatch(r"(\d+)(?:\.\.=?(\d+))?", token)
        if not m:
            fail(f"`matches!(self.{field}, …)` 里出现本守卫读不懂的形态 `{token}` ⇒ 别让它静默少读")
        low = int(m.group(1))
        high = int(m.group(2)) if m.group(2) else low
        values.extend(range(low, high + 1))
    return values


# ---------------------------------------------------------------------------
# 驱动
# ---------------------------------------------------------------------------

def compare(label: str, impl: list, contract: list, lower_bound: int, pointer: str) -> None:
    impl_set, contract_set = set(impl), set(contract)
    print(
        f"[读数] `{label}`：实现侧 {len(impl_set)} 个（{', '.join(map(str, sorted(impl_set)))}）"
        f"／契约侧 {len(contract_set)} 个（{', '.join(map(str, sorted(contract_set)))}）"
    )
    # ⚠ 顺序是**读数顺序**的一部分：集合差要先于下界。
    # 先查下界会让"契约少一项（N-1/N）"报成"被掏空了？"—— 把**少了哪个名字**这条最有用的信息
    # 埋在一条粗报警后面。下界只在该查"两边同时删空"（差集为空、等式空转）时才不可替代。
    dup = sorted({c for c in contract if contract.count(c) > 1}, key=str)
    if dup:
        fail(f"`{label}` 的契约 enum 有重复条目: {', '.join(map(str, dup))}")
    missing = sorted(impl_set - contract_set, key=str)
    if missing:
        fail(
            f"`{label}`：模型里有、schema 里没登记 ⇒ `.yeban` 归档会**静默拒绝**合法工程: "
            f"{', '.join(map(str, missing))}"
        )
    extra = sorted(contract_set - impl_set, key=str)
    if extra:
        fail(f"`{label}`：schema 里登记了、模型里没有（幽灵条款）: {', '.join(map(str, extra))}")
    if len(impl_set) < lower_bound:
        fail(f"`{label}` 实现侧只剩 {len(impl_set)} 个（下界 {lower_bound}）—— 模型名册被掏空了？")
    if len(contract_set) < lower_bound:
        fail(
            f"`{label}` 契约侧只剩 {len(contract_set)} 个（下界 {lower_bound}），指针 `{pointer}`"
            " —— 两边同名但被成对删空，等式空转 ⇒ 看下界"
        )


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    if "--root" in sys.argv:
        root = Path(sys.argv[sys.argv.index("--root") + 1]).resolve()

    schema_path = root / "schemas" / "project.schema.json"
    if not schema_path.is_file():
        fail(f"契约文件不存在: {schema_path}")
    schema = json.loads(schema_path.read_text(encoding="utf-8"))

    sources: dict[str, str] = {}
    total = 0
    for label, kind, rel, args, pointer, lower_bound in SITES:
        src = root / rel
        if not src.is_file():
            fail(f"实现侧文件不存在: {src}（站点 `{label}`）")
        text = sources.setdefault(rel, src.read_text(encoding="utf-8"))
        contract = resolve(schema, pointer)
        if kind == "variants":
            impl, _ = enum_body(text, args[0])
        elif kind == "int_serde":
            impl = int_vocabulary(text, args[0], args[1], args[2])
        elif kind == "matches":
            impl = matches_vocabulary(text, args[0])
        else:
            fail(f"站点 `{label}` 的生产者类型 {kind!r} 本守卫不认（拼错了？）")
        compare(label, impl, contract, lower_bound, pointer)
        total += len(set(impl))

    covered = len(SITES)
    print(
        f"[ok] {covered} 处模型名册的实现与契约**集合相等**（合计 {total} 项，各处 ≥ 登记下界）"
        f" ⇒ `schemas/project.schema.json` 不再是不对账的手抄"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
