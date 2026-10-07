#!/usr/bin/env python3
"""钉住 `yeban-mcp` 的**依赖方向**规则。

`crates/yeban-mcp/Cargo.toml` 自己写着"**轻量 crate**: 不拖音频栈进 MCP"，而本会话曾两次
差点违反它（想给 MCP 加 `yeban-app` 以取音符构造；想加 `yeban-render` 以取 SMF 编码器）。
规则写在注释里**挡不住**下一次，所以要有一个会失败的检查（账本第 261/273 轮）。

判据：本 crate 的依赖段里出现任何被禁 crate ⇒ 非零退出并指名。用法:

    python3 scripts/gates/check_mcp_dependency_direction.py
    python3 scripts/gates/check_mcp_dependency_direction.py --manifest <Cargo.toml>   # 只测某份清单
    python3 scripts/gates/check_mcp_dependency_direction.py --self-test              # 自带牙测（不读真清单）

`--manifest` 是**诊断/牙测**入口：它把同一份黑名单套到你指的那份清单上（报错点名的是
那份清单自己的 crate 名）。门禁（`run-gates.sh light`）**不带**该参数，只查
`crates/yeban-mcp/Cargo.toml` 这一份。

## 怎么取"依赖名"（第 1 版这里有一个**真缺陷**，已修，勿退回）

第 1 版的键提取是 `line.split("=", 1)[0].strip()`。本仓的成员依赖写法是
`yeban-render.workspace = true`（根 `Cargo.toml` 明文规定"成员 crate 一律使用
`foo.workspace = true`"），于是那条式子给出 `"yeban-render.workspace"`，**永不等于**黑名单
里的 `"yeban-render"` ⇒ 守卫对"用本仓标准写法加的禁用依赖"是**瞎的**，且照样打印 `[ok]`。
实测：当时 `crates/yeban-mcp/Cargo.toml:73` 的 `[dependencies]` 里**已经有**
`yeban-render.workspace = true`，守卫仍报 `[ok] … 5 个被禁 crate 均未被直接依赖`。
⇒ 现在改成**按 TOML 表格结构逐段扫描**（见 `scan_dependencies`），键 = 真正的依赖名。

## 为什么不是 `tomllib`

标准库的 `tomllib` 能解析整份清单，但**给不出行号**，而本仓的报错纪律要求指到
`file:line`；它也**不会**告诉你"这里有一条我看不懂的声明"，只会整份解析失败（于是要么
假红整份清单、要么被迫退化成 try/except 后静默跳过 —— 后者正是本仓最怕的假绿）。
因此这里用一段**只面向依赖表的小型文本扫描器**：不新增依赖、不做完整 TOML 解析，
但①按表格结构（而不是正则猜键）取依赖名、②内联表跨行按花括号配平、③认不出的行
**逐行报错**而不是静默跳过。

## 已登记例外（REGISTERED EXCEPTION）—— 不是被弱化的黑名单

判据修好之后，本守卫在 `crates/yeban-mcp/Cargo.toml` 上**合法地为红**：那里确实有一条
被禁的依赖边（`yeban-render`，见 `docs/ledger/mcp-tools-expansion-notes.md` §10.4 / §11）。
判据正确不等于这条边正确 —— 但也不能为了让门禁变绿就把它从黑名单里删掉（那是**真的**
弱化判据）。所以本守卫带一张**逐字登记**的例外表 `EXCEPTIONS`：每条例外的键是
`(crate, 被禁依赖名)` 这一对**字面量**；命中 ⇒ 容忍但**每次运行都打印**；没命中 ⇒
陈旧 ⇒ **红**。

**它不是什么**（由 `validate_exception_registry` 与 `resolve_hits` 机械保证，
`--self-test` 逐条注入反例）：
- 不是通配符、不是正则：`crate` / `dep` 必须是单个 `[A-Za-z0-9_-]+` 字面量，`dep` 还必须
  **逐字等于** `FORBIDDEN` 里的一项（通配、正则、或指向一条并不被禁的依赖 ⇒ exit=2）；
- 不是"跳过整个 crate"：例外只对它登记的那**一对**生效，同一个 crate 上的**第二条**
  被禁依赖照红不误；
- 不是"躲在射程外的白名单"：例外必须登记在本门禁**真正检查**的那个 crate 上；登记在
  别的 crate 上 ⇒ exit=2（那种例外每次运行都"不在射程"，陈旧永远没机会被发现）；
- 不是永久豁免：例外记录的那条边一旦消失，例外就是**陈旧**的 ⇒ exit=1，必须与违规在
  **同一次改动**里一起删掉。

**fail-closed 规则（这里选定的最强一条）**：一条例外要在**三个方向**上都还活着才算数 ——
① `FORBIDDEN` 里仍写着 `dep`（否则 `validate_exception_registry` 报错，exit=2）；
② 登记在**本门禁真正检查**的那个 crate 上（登记在别处 ⇒ 每次运行都"不在射程"、陈旧永远
没机会被发现 ⇒ 同样 exit=2）；
③ 被检查的清单里**真的**还有这条声明（否则 `resolve_hits` 判它陈旧，exit=1）。
任一方向断掉，这条例外都**不能再放行任何东西**。于是例外既不能悄悄扩大（加 `*`、加正则、
换 crate、挪到射程外），也不能在违规被修好之后悄悄留下 —— 它必须与它所记录的违规同生共死。
刻意**不**在例外里记录行号：行号引用在本仓反复腐烂（见 `check_feature_alignment.py` 判据 6），
而依赖名是稳定的键；当前行号由守卫每次运行**现场打印**，既精确又不会腐烂。
"""
from __future__ import annotations

import argparse
import dataclasses
import pathlib
import re
import sys

#: 被禁的直接依赖：带 GUI / 音频栈 / 渲染栈的 crate。MCP 要这些东西时，
#: 正确做法是**把共享件下移到 model 级 crate**，而不是加依赖（账本第 261 轮）。
#:
#: ⚠ 本文件**不承诺**任何固定行号：`docs/ledger/mcp-tools-expansion-notes.md` §10.4/§11
#: 引用本守卫时按**节名与符号名**（`FORBIDDEN` / `scan_dependencies`）而不是行号 ——
#: 本仓已经吃过"引用行号 → 文件一改就腐烂"的账（见 `check_feature_alignment.py` 判据 6）。
FORBIDDEN = (
    "yeban-app",      # 带 GUI（Slint）
    "yeban-dsp",      # 音频栈
    "yeban-render",   # 依赖 dsp + hound
    "yeban-engine",   # 设备/回调
    "yeban-audio",    # 音频 I/O
)

REPO = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = pathlib.Path("crates/yeban-mcp/Cargo.toml")


# ---------------------------------------------------------------------------
# 已登记例外：逐字登记的一对 (crate, 被禁依赖) + 它的依据与来源
# ---------------------------------------------------------------------------

@dataclasses.dataclass(frozen=True)
class RegisteredException:
    """一条**逐字登记**的例外。

    键是 `(crate, dep)` 这两个字面量 —— 没有通配、没有正则、没有"整个 crate 放行"。
    另外带上"谁给的依据 / 从哪来 / 为什么容忍 / 什么条件下必须删掉"，因为一条**没有**
    这四个字段的例外无法被审查，只能腐烂成一张空白名单。
    """

    crate: str                   # 违规的 crate（清单里的 `[package] name`）
    dep: str                     # 被禁依赖名；必须**逐字等于** FORBIDDEN 里的一项
    authority: str               # 谁给了这次容忍（规则出处）
    provenance: tuple[str, ...]  # 证据链：自述规则 / 引入提交 / 账本轮次 / 笔记
    tolerated_because: str       # 为什么现在容忍它
    removal_condition: str       # 什么条件下这条例外必须消失（不消失就陈旧 ⇒ 红）


#: 当前**唯一**一条已登记例外。它不是白名单，是一张欠条：已知偏离 + 责任线索 + 失效条件。
EXCEPTIONS: tuple[RegisteredException, ...] = (
    RegisteredException(
        crate="yeban-mcp",
        dep="yeban-render",
        authority=(
            'crates/yeban-mcp/Cargo.toml:30-31 的 crate 自述规则「轻量 crate: 不拖音频栈进 MCP」'
            "—— 本次容忍**没有**废除这条规则，只是把它的一次已知偏离记在案上"
        ),
        provenance=(
            "引入该依赖边的提交 f11de2c feat(mcp-render): yeban_render_master 真渲染接线 [MCP-TOOL-008]",
            "crates/yeban-render/Cargo.toml:40-56 显示 yeban-render -> yeban-dsp + hound"
            "（正是自述规则点名要挡的音频栈）",
            "docs/DEVELOPMENT_LEDGER.md Round 261：把 MCP 需要的共享件下移到 model 级 crate",
            "docs/DEVELOPMENT_LEDGER.md Round 273：SMF 编码器按同一手法下移成 yeban-midi",
            "docs/ledger/mcp-tools-expansion-notes.md §10.4 / §11：缺口登记与判据修复记录",
        ),
        tolerated_because=(
            "yeban_render_master 的真渲染接线已经落地并依赖这条边；在集成者把"
            "「按 Round 261/273 把共享件下移」与「由负责人正式裁决 MCP 可依赖 yeban-render」"
            "二者之一执行完之前，本例外让门禁保持可运行的绿灯，同时把这条已知偏离显式"
            "打印在每一次运行的输出里（而不是让它退回静默放行）。"
        ),
        removal_condition=(
            "该依赖边一旦被移除（下移完成），或 FORBIDDEN 里不再有 yeban-render"
            "（负责人裁决后改黑名单 / 改那句自述规则），本例外即陈旧或失效"
            " ⇒ 本守卫 exit=1 / exit=2，必须与那次改动同一次删掉本条目。"
        ),
    ),
)


def main() -> int:
    parser = argparse.ArgumentParser(description="yeban-mcp 依赖方向守卫")
    parser.add_argument("--manifest", default=None, help="要检查的 Cargo.toml（默认 crates/yeban-mcp/Cargo.toml）")
    parser.add_argument("--self-test", action="store_true", help="只跑内置牙测（在临时目录里注入/放行，不读真清单）")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    path = pathlib.Path(args.manifest) if args.manifest else MANIFEST
    if not path.is_absolute():
        path = REPO / path
    return check(path)


# ---------------------------------------------------------------------------
# 依赖表扫描器：键 = 真正的依赖名
# ---------------------------------------------------------------------------

_DEP_HEADER = re.compile(
    r"^(?P<ws>workspace\s*\.\s*)?"
    r"(?:(?P<cfg>target\s*\.\s*'[^']*'\s*)\.\s*)?"
    # ⚠ 末尾的 `(?![-\w])` 是承重的：`re.match` 不要求吃掉整行，所以没有它时
    # 形如 `[dependencies-extra]` 的表头会被当成合法的依赖段（凭空多出一个射程）。
    r"(?P<kind>dependencies|dev-dependencies|build-dependencies)(?![-\w])"
    # `[dependencies.foo]` 子表形态（与 Cargo 接受的 `[dependencies.foo] path = "..."` 同构）
    r"(?:\.\s*(?P<sub>.+?))?$"
)
_HEADER = re.compile(r"^\[\[?\s*(?P<inner>.+?)\s*\]?\]$")
#: 点号继承写法 `name.workspace = true` / `name.path = "..."` / `name.optional = true`
_DOTTED = re.compile(r'^(?P<name>"(?:[^"\\]|\\.)*"|[A-Za-z0-9_\-]+)\s*\.\s*(?P<prop>[A-Za-z0-9_\-]+)\s*=')
#: 重命名依赖 `package = "真正包名"`
_PACKAGE = re.compile(r'\bpackage\s*=\s*"([^"]+)"')


def _strip_comment(text: str) -> str:
    """去掉行尾 `#` 注释，**尊重引号**（`features = ["a#b"]` 里的 `#` 不是注释）。"""
    out: list[str] = []
    quote: str | None = None
    esc = False
    for ch in text:
        if quote:
            out.append(ch)
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == quote:
                quote = None
        elif ch in "\"'":
            quote = ch
            out.append(ch)
        elif ch == "#":
            break
        else:
            out.append(ch)
    return "".join(out)


def _brace_delta(text: str) -> int:
    """一段文本里 `{}` 的净深度，忽略引号内。"""
    delta = 0
    quote: str | None = None
    esc = False
    for ch in text:
        if quote:
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == quote:
                quote = None
        elif ch in "\"'":
            quote = ch
        elif ch == "{":
            delta += 1
        elif ch == "}":
            delta -= 1
    return delta


def scan_dependencies(text: str, *, include_catalog: bool = False):
    """扫出依赖声明。返回 `(declarations, unjudged, sections)`。

    `declarations` = `[(lineno, key, package_or_None, section)]`，**每条声明的键就是
    依赖名本身**（点号写法取第一个点号之前的部分；重命名依赖另带 `package`）。
    `unjudged` = 认得出"这里有一条声明"但读不出键的行 —— 逐行报错，不静默通过。
    `sections` = 出现过的依赖段名（见证"真的读到了依赖段"，否则 0 命中是空判据）。

    覆盖的写法（本仓实际出现的都覆盖，见 `--self-test` 的注入/放行用例）：
      `name = "..."` / `name = { ... }`（可跨行）/ `name.workspace = true`
      / `name = { workspace = true, ... }` / `[dependencies.name]` 子表
      / `package = "..."` 重命名 / `optional = true` / `features = [...]`
      / `[dependencies]` / `[dev-dependencies]` / `[build-dependencies]`
      / `[target.'cfg(...)'.dependencies]`
    """
    declarations: list[tuple[int, str, str | None, str]] = []
    unjudged: list[tuple[int, str]] = []
    sections: list[str] = []

    lines = text.splitlines()
    section: str | None = None      # 当前依赖段名（含 cfg 前缀），非依赖段为 None
    sub_table: str | None = None    # `[dependencies.foo]` 的 foo
    i = 0
    while i < len(lines):
        stripped = _strip_comment(lines[i]).strip()
        lineno = i + 1
        if not stripped:
            i += 1
            continue

        if stripped.startswith("["):
            is_array = stripped.startswith("[[")
            header = _HEADER.match(stripped)
            inner = header.group("inner") if header else stripped.strip("[]")
            match = _DEP_HEADER.match(inner)
            section = None
            sub_table = None
            if match:
                # `[workspace.dependencies]` 是**版本目录**（可被任何成员继承），
                # 不是本 crate 的依赖声明 ⇒ 默认不判（include_catalog 只给自测用）。
                # 用正则捕获组而不是 `inner.startswith("workspace.")`：后者对
                # `workspace . dependencies`（点号两侧带空格）会漏判。
                catalog = match.group("ws") is not None
                if not (catalog and not include_catalog):
                    section = inner
                    sections.append(inner)
                    sub = match.group("sub")
                    sub = sub.strip().strip('"').strip("'") if sub else None
                    if is_array:
                        unjudged.append((lineno, f"[[{inner}]] 数组表形态: {stripped}"))
                    elif sub:
                        sub_table = sub
                        declarations.append((lineno, sub, None, inner))
            i += 1
            continue

        if section is None or sub_table is not None:
            # 非依赖段，或 `[dependencies.foo]` 子表的表体（属性行，不构成新声明）
            i += 1
            continue

        dotted = _DOTTED.match(stripped)
        if dotted:
            declarations.append((lineno, dotted.group("name").strip('"'), None, section))
            i += 1
            continue

        if "=" not in stripped:
            unjudged.append((lineno, stripped))
            i += 1
            continue

        key, rhs = stripped.split("=", 1)
        key = key.strip().strip('"')
        rhs = rhs.strip()
        if not key:
            unjudged.append((lineno, stripped))
            i += 1
            continue

        if rhs.startswith("{"):
            # 内联表**可以跨行**（features 数组就是），按花括号配平吃掉续行；
            # 续行不是新声明 —— 这正是"按结构而不是按行"的关键。
            depth = _brace_delta(rhs)
            j = i
            while depth > 0 and j + 1 < len(lines):
                j += 1
                depth += _brace_delta(_strip_comment(lines[j]))
            blob = " ".join(_strip_comment(x) for x in lines[i : j + 1])
            package = _PACKAGE.search(blob)
            declarations.append((lineno, key, package.group(1) if package else None, section))
            i = j + 1
            continue

        if rhs[:1] in "\"'" or rhs.startswith("["):
            # 字符串形态，或 path 数组形态（ADR-0001 D21 的 wildcard path）
            declarations.append((lineno, key, None, section))
            i += 1
            continue

        unjudged.append((lineno, stripped))
        i += 1

    return declarations, unjudged, sections


_IDENT = re.compile(r"^[A-Za-z0-9_-]+$")


def _manifest_package_name(path: pathlib.Path) -> str | None:
    """读一份清单的 `[package] name`（读不到就 None）。"""
    if not path.is_file():
        return None
    match = re.search(r'(?m)^\s*name\s*=\s*"([^"]+)"', path.read_text(encoding="utf-8"))
    return match.group(1) if match else None


def validate_exception_registry(exceptions, scope_crate: str | None = None) -> list[str]:
    """例外表自身的 fail-closed 校验。返回错误列表（空 = 合法）。

    这些判据就是"例外表不许腐烂"的机械保证：**只要有一条不合法**，`check()` 直接
    exit=2，于是**一条**违规都放行不了。任何放宽——通配/正则键、指向黑名单以外的依赖、
    重复的 `(crate, dep)`、空的依据/来源/理由/失效条件——都在这里被挡住，而不是靠自觉。

    `scope_crate` = 本门禁**真正检查**的那个 crate（默认清单的 `[package] name`）。
    非 None 时，登记在别的 crate 上的例外一律拒绝：那种例外在门禁运行中**永远是"不在
    射程"** ⇒ 它的陈旧永远没机会被发现 ⇒ 它是一条躲在射程外的潜在放行。拒绝它。
    """
    errors: list[str] = []
    seen: set[tuple[str, str]] = set()
    for i, e in enumerate(exceptions, 1):
        tag = f"例外 #{i} ({e.crate} → {e.dep})"
        for field, value in (("crate", e.crate), ("dep", e.dep)):
            if not isinstance(value, str) or not _IDENT.match(value):
                errors.append(f"{tag}: {field}={value!r} 不是单个字面标识符 ⇒ 禁通配/正则/空值")
        if not isinstance(e.dep, str) or e.dep not in FORBIDDEN:
            errors.append(f"{tag}: dep={e.dep!r} 不在 FORBIDDEN 里 ⇒ 例外指向一条并不被禁的依赖（陈旧配置）")
        if scope_crate is not None and e.crate != scope_crate:
            errors.append(
                f"{tag}: 登记在门禁**并不检查**的 crate 上（门禁只查 {scope_crate}）"
                " ⇒ 这条例外每次运行都'不在射程'，陈旧永远没机会被发现"
            )
        key = (e.crate, e.dep)
        if key in seen:
            errors.append(f"{tag}: (crate, dep) 与前面的条目重复 ⇒ 一对依赖只能登记一次")
        seen.add(key)
        for field, value in (
            ("authority", e.authority),
            ("tolerated_because", e.tolerated_because),
            ("removal_condition", e.removal_condition),
        ):
            if not isinstance(value, str) or not value.strip():
                errors.append(f"{tag}: {field} 为空 ⇒ 例外必须写清依据 / 理由 / 失效条件")
        if not e.provenance or not all(isinstance(p, str) and p.strip() for p in e.provenance):
            errors.append(f"{tag}: provenance 为空 ⇒ 例外必须带来源（自述规则 / 引入提交 / 账本轮次）")
    return errors


def resolve_hits(crate: str, hits, exceptions):
    """把命中拆成 `(excused, violations, stale)`。

    - `excused`    = 命中且**逐字**匹配到一条例外的 `(hit, exception)`；
    - `violations` = 命中但没有任何例外认领它（**包括**同一个 crate 上的第二对）；
    - `stale`      = 登记在**本 crate** 上、但本次一条都没命中的例外 ⇒ 调用方必须报红。

    只按 `(crate, 被禁依赖名)` 做**字面**配对：没有前缀匹配、没有 `fnmatch`、没有正则，
    更没有"这个 crate 在例外表里出现过就整份跳过"。
    """
    by_key = {(e.crate, e.dep): e for e in exceptions}
    excused: list[tuple[tuple, RegisteredException]] = []
    violations: list[tuple] = []
    live: set[tuple[str, str]] = set()
    for hit in sorted(hits):
        key = (crate, hit[4])
        exception = by_key.get(key)
        if exception is None:
            violations.append(hit)
        else:
            excused.append((hit, exception))
            live.add(key)
    stale = [e for e in exceptions if e.crate == crate and (e.crate, e.dep) not in live]
    return excused, violations, stale


def print_exception_registry(path: pathlib.Path, crate: str, excused, stale, registry) -> None:
    """每一次运行都打印**实际生效**的那张例外表 —— 例外**不允许**安静地存在。

    无论本次是绿还是红、命中还是没命中，读者都能从输出里看到：存在例外、它是哪一对、
    凭什么容忍、从哪来、什么条件下必须删掉。`[ok]` 行本身也会点名被容忍的依赖。
    """
    if not registry:
        return
    hit_at = {(e.crate, e.dep): h for h, e in excused}
    stale_keys = {(e.crate, e.dep) for e in stale}
    print(
        f"[EXC] 已登记例外 {len(registry)} 条：按键 (crate, 被禁依赖) 逐字精确匹配，"
        "不是通配、不是正则、不是「跳过整个 crate」"
    )
    for i, e in enumerate(registry, 1):
        if (e.crate, e.dep) in hit_at:
            h = hit_at[(e.crate, e.dep)]
            state = f"本次命中 {_rel(path)}:{h[0]} [{h[3]}]"
        elif (e.crate, e.dep) in stale_keys:
            state = "本次**陈旧**（清单里已无此声明）⇒ 见下方 FAIL"
        else:
            state = f"本次不在射程（检查的是 {crate}）"
        print(f"[EXC]   #{i} {e.crate} → {e.dep}   [{state}]")
        print(f"[EXC]        依据: {e.authority}")
        for item in e.provenance:
            print(f"[EXC]        来源: {item}")
        print(f"[EXC]        容忍理由: {e.tolerated_because}")
        print(f"[EXC]        失效条件: {e.removal_condition}")


def check(path: pathlib.Path, crate: str | None = None, exceptions=None) -> int:
    # `crate` 只影响报错时点名的对象。默认清单永远是 `crates/yeban-mcp/Cargo.toml`，
    # 但 `--manifest`（牙测用）可能指向别处 ⇒ 报错**必须点名它真正读的那份清单的 crate**，
    # 否则一份 `yeban-app` 的清单会打印"yeban-mcp 直接依赖了被禁 crate"这种**假归属**。
    exs = EXCEPTIONS if exceptions is None else tuple(exceptions)

    # ⓪ 例外表自身先过合法性：一张会腐烂的例外表比没有例外表更坏。
    #    不合法 ⇒ exit=2（守卫配置错误，与"发现违规"的 exit=1 区分），**绝不**进入绿的路径。
    #    scope_crate = 本门禁真正检查的那个 crate ⇒ 登记在别的 crate 上的例外直接被拒
    #    （那种例外永远是"不在射程"，它的陈旧没机会被发现）。
    registry_errors = validate_exception_registry(exs, scope_crate=_manifest_package_name(MANIFEST))
    if registry_errors:
        print("[FAIL] 已登记例外表本身不合法 ⇒ 拒绝用它放行任何依赖:", file=sys.stderr)
        for message in registry_errors:
            print(f"        {message}", file=sys.stderr)
        return 2

    if not path.is_file():
        print(f"[FAIL] 找不到清单 {path}", file=sys.stderr)
        return 1
    text = path.read_text(encoding="utf-8")
    if crate is None:
        package = re.search(r'(?m)^\s*name\s*=\s*"([^"]+)"', text)
        crate = package.group(1) if package else _rel(path)
    declarations, unjudged, sections = scan_dependencies(text)

    # ① 判不了的一定要说（本仓纪律：静默跳过比慢更坏）
    if unjudged:
        for lineno, raw in unjudged:
            print(f"[FAIL] {_rel(path)}:{lineno} 认得出是依赖段里的一行, 但读不出依赖名: {raw}", file=sys.stderr)
        return 1

    # ② 见证：真的读到了依赖段与声明，否则"0 命中"是空判据，不是绿
    if not sections:
        print(f"[FAIL] {_rel(path)} 里没有找到任何依赖段 ([dependencies] 一族) ⇒ 判据空转", file=sys.stderr)
        return 1
    if not declarations:
        print(f"[FAIL] {_rel(path)} 的依赖段里读到 0 条声明 ⇒ 判据可能空转, 不能当作已通过", file=sys.stderr)
        return 1

    # ③ 判据：键（或重命名依赖的真实包名）命中黑名单；把命中的那个禁用名逐条带出来，
    #    因为例外是按"命中名"配对的，而 key / package 未必等于它。
    hits: list[tuple[int, str, str, str, str]] = []   # (lineno, key, package, section, forbidden)
    for lineno, key, package, section in declarations:
        for name in sorted({key, package} - {None}):
            if name in FORBIDDEN:
                hits.append((lineno, key, package or "", section, name))

    # ④ 例外只认它逐字登记的那一对；登记了却没命中的（陈旧）同样要报
    excused, violations, stale = resolve_hits(crate, hits, exs)

    # ⑤ 例外**每次运行都打印**（绿/红两条路径都会走到这里）：读者必须看到"存在例外、
    #    它是哪一对、凭什么、何时失效"，而不是只看到一个 [ok]。打印的是**本次生效**的
    #    那张表 `exs`，不是模块全局 —— 否则一张被覆盖的表会打印出并未生效的条目。
    print_exception_registry(path, crate, excused, stale, exs)

    renamed = [(ln, k, p, s) for ln, k, p, s in declarations if p and p != k]

    failed = False
    if violations:
        failed = True
        print(f"[FAIL] {crate} 直接依赖了被禁 crate（且未登记例外）:", file=sys.stderr)
        for lineno, key, package, section, _forbidden in sorted(violations):
            shown = key + (f"（重命名自 `{package}`）" if package and package != key else "")
            print(f"        {_rel(path)}:{lineno} [{section}] {shown}", file=sys.stderr)
        print("       ⚠ 已登记例外只对**它逐字记录的那一对** (crate, 被禁依赖) 生效，不覆盖这一条。", file=sys.stderr)
        print("       规则见 crates/yeban-mcp/Cargo.toml 的'轻量 crate: 不拖音频栈进 MCP'；", file=sys.stderr)
        print("       正确做法是把共享件下移到 model 级 crate（账本第 261/273 轮）。", file=sys.stderr)

    if stale:
        failed = True
        print("[FAIL] 已登记例外**陈旧**：它记录的违规在本清单里已经不存在 ⇒", file=sys.stderr)
        print("       例外必须与它记录的违规同生共死；请在同一次改动里删掉这条登记：", file=sys.stderr)
        for e in stale:
            print(f"        {e.crate} → {e.dep}（{_rel(path)} 的依赖段里没有任何声明命中 {e.dep}）", file=sys.stderr)

    if failed:
        return 1

    detail = ""
    if renamed:
        detail = "；重命名依赖 " + ", ".join(
            f"{k}→{p}({_rel(path)}:{ln})" for ln, k, p, _s in renamed
        )
    excused_deps = ", ".join(e.dep for _h, e in excused)
    print(
        f"[ok] mcp-dependency-direction: {crate}: 被禁 {len(FORBIDDEN)} 个 crate 里 "
        f"{len(violations)} 条未登记违规、{len(excused)} 条已登记例外"
        + (f"（{excused_deps}）" if excused else "")
        + f"；读了 {len(declarations)} 条声明 / {len(sections)} 个依赖段{detail}"
    )
    return 0


def _rel(path: pathlib.Path) -> str:
    try:
        return str(path.resolve().relative_to(REPO))
    except ValueError:
        return str(path)


# ---------------------------------------------------------------------------
# 牙测（--self-test）：注入必须红、放行必须绿、判不了必须说
# ---------------------------------------------------------------------------

_SELF_CRATE = "yeban-mcp-selftest"

#: 牙测用的**合成** crate 名。刻意**不叫** `yeban-mcp`：那名字上挂着一条已登记例外
#: （`yeban-render`），如果所有放行用例都用真名，它们会因为"例外陈旧"而红 —— 那测到的
#: 就不是"放行"，而是"陈旧"。真名 + 真例外由下面 `_SELF_MCP` 的几条用例单独覆盖。
_SELF_BASE = f"""[package]
name = "{_SELF_CRATE}"
version.workspace = true

[dependencies]
yeban-midi = {{ path = "../yeban-midi" }}
serde.workspace = true
yeban-model.workspace = true
libm.workspace = true

[dev-dependencies]
proptest.workspace = true
"""

#: 真 crate 名（带已登记例外）的清单，用来测例外机制本身。
_SELF_MCP = _SELF_BASE.replace(f'name = "{_SELF_CRATE}"', 'name = "yeban-mcp"')


def _run_check_on(text: str, tmp: pathlib.Path, name: str, exceptions=None) -> tuple[int, str]:
    import contextlib
    import io

    manifest = tmp / name
    manifest.write_text(text, encoding="utf-8")
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = check(manifest, exceptions=exceptions)
    return code, out.getvalue() + err.getvalue()


def self_test() -> int:
    import tempfile

    ok_fixture = f"{_SELF_CRATE}: 被禁 {len(FORBIDDEN)} 个 crate 里 0 条未登记违规、0 条已登记例外"

    # (name, manifest text, 期望 exit, 输出里必须出现的子串, 例外表覆盖 or None)
    cases: list[tuple[str, str, int, tuple[str, ...], tuple | None]] = [
        # --- 注入：必须红（fixture crate 名，不带任何已登记例外） ---
        # 本仓标准写法（点号继承）—— 修复前这一条**不会**被抓住
        ("dotted-forbidden", _SELF_BASE.replace("libm.workspace = true", "yeban-dsp.workspace = true"), 1, ("[dependencies] yeban-dsp",), None),
        ("inline-table-forbidden", _SELF_BASE.replace("libm.workspace = true", "yeban-app = { workspace = true }"), 1, ("yeban-app",), None),
        ("path-table-forbidden", _SELF_BASE.replace("libm.workspace = true", 'yeban-dsp = { path = "../yeban-dsp" }'), 1, ("yeban-dsp",), None),
        ("dev-deps-forbidden", _SELF_BASE.replace("proptest.workspace = true", "yeban-engine.workspace = true"), 1, ("yeban-engine",), None),
        ("target-section-forbidden", _SELF_BASE + "\n[target.'cfg(unix)'.dependencies]\nyeban-dsp.workspace = true\n", 1, ("yeban-dsp",), None),
        ("subtable-forbidden", _SELF_BASE + '\n[dependencies.yeban-render]\npath = "../yeban-render"\n', 1, ("yeban-render",), None),
        ("renamed-forbidden", _SELF_BASE + '\nalias = { package = "yeban-render", path = "../yeban-render" }\n', 1, ("yeban-render",), None),
        ("plain-string-forbidden", _SELF_BASE.replace("libm.workspace = true", 'yeban-audio = "0.1"\n# 注释里的 yeban-engine 不算'), 1, ("yeban-audio",), None),
        # --- 放行：合法形态必须绿 ---
        ("legit-dotted", _SELF_BASE, 0, (ok_fixture,), None),
        ("legit-inline-workspace", _SELF_BASE.replace("serde.workspace = true", "serde = { workspace = true }"), 0, (ok_fixture,), None),
        ("legit-inline-path", _SELF_BASE.replace('yeban-midi = { path = "../yeban-midi" }', 'yeban-midi = { path = "../yeban-midi", features = ["std"] }'), 0, (ok_fixture,), None),
        # 多行 features 内联表：续行不得被当成新声明（按行取键会造出 `"std"` 这样的键）
        ("legit-multiline", _SELF_BASE.replace("libm.workspace = true", 'slint = { workspace = true, features = [\n    "std",\n    "compat-1-2",\n] }'), 0, (ok_fixture,), None),
        # 注释里的禁用名不算（整段子串匹配会假红；这是本守卫第一版就被牙测抓到过的坑）
        ("legit-comment-mention", _SELF_BASE.replace("libm.workspace = true", '# 为什么不是 yeban-render: 它拖音频栈\nlibm.workspace = true'), 0, (ok_fixture,), None),
        # `[dependencies-extra]` **不是**依赖段（表头正则末尾的 `(?![-\w])` 是承重的）：
        # 少了它，段头匹配会把这一节当射程，于是凭空造出一条并不存在的禁用依赖。
        ("dependencies-extra-is-not-a-section", _SELF_BASE + "\n[dependencies-extra]\nyeban-dsp.workspace = true\n", 0, (ok_fixture,), None),
        # --- 已登记例外：放行但**每次可见**；同一 crate 上的第二条照红；陈旧必红；坏表闭死 ---
        ("registered-exception-passes", _SELF_MCP.replace("libm.workspace = true", "yeban-render.workspace = true"), 0,
         ("1 条已登记例外", "yeban-render", "f11de2c", "0 条未登记违规", "[EXC]"), None),
        ("second-forbidden-same-crate-still-red", _SELF_MCP.replace("libm.workspace = true", "yeban-render.workspace = true\nyeban-dsp.workspace = true"), 1,
         ("未登记例外", "yeban-dsp", "yeban-render"), None),
        ("stale-exception-fails", _SELF_MCP, 1, ("陈旧", "yeban-render"), None),
        ("invalid-registry-fails-closed", _SELF_BASE, 2, ("例外表本身不合法",), (dataclasses.replace(EXCEPTIONS[0], dep="yeban-*"),)),
        ("out-of-scope-exception-fails-closed", _SELF_BASE, 2, ("例外表本身不合法", "并不检查"), (dataclasses.replace(EXCEPTIONS[0], crate="yeban-app"),)),
        # --- 判不了必须说 / 空判据不能当绿 ---
        ("empty-deps-section", f'[package]\nname = "{_SELF_CRATE}"\n\n[dependencies]\n', 1, ("0 条声明",), None),
        ("no-deps-section", f'[package]\nname = "{_SELF_CRATE}"\n', 1, ("没有找到任何依赖段",), None),
        ("unjudgeable-no-equals", _SELF_BASE + "\nnot-an-assignment\n", 1, ("读不出依赖名",), None),
        ("unjudgeable-array-table", _SELF_BASE + "\n[[dependencies]]\n", 1, ("读不出依赖名", "数组表形态"), None),
    ]

    failures: list[str] = []
    with tempfile.TemporaryDirectory() as tmpdir:
        tmp = pathlib.Path(tmpdir)
        for name, text, want_code, want_texts, override in cases:
            code, output = _run_check_on(text, tmp, f"{name}.toml", override)
            if code != want_code:
                failures.append(f"{name}: 期望 exit={want_code}, 实得 exit={code}\n{output}")
                continue
            for want_text in want_texts:
                if want_text not in output:
                    failures.append(f"{name}: 输出里没有出现 {want_text!r}\n{output}")
                    break

    # 扫描器与例外表的"结构"判据（不经文件系统，直接调函数）。
    # ① `[workspace.dependencies]` 是版本目录，默认**不是**成员依赖 ⇒ 0 条声明；
    # ② 打开 include_catalog（只给测试用）才看得到它 ⇒ 1 条声明。
    # 没有这条，`include_catalog` 就是死参数，而"目录被当成依赖"这种假红也没东西挡。
    structural: list[tuple[str, bool]] = []
    catalog = '[workspace.dependencies]\nyeban-render = { path = "crates/yeban-render" }\n'
    structural.append(("workspace-catalog-default-0-declarations", len(scan_dependencies(catalog)[0]) == 0))
    structural.append(("workspace-catalog-included-1-declaration", len(scan_dependencies(catalog, include_catalog=True)[0]) == 1))

    # 例外机制的三条结构判据：逐字配对 / 陈旧必被标出 / 只对本 crate 生效
    probe = [
        (73, "yeban-render", "", "[dependencies]", "yeban-render"),
        (79, "yeban-dsp", "", "[dependencies]", "yeban-dsp"),
    ]
    excused, violations, stale = resolve_hits("yeban-mcp", probe, EXCEPTIONS)
    structural.append((
        "exception-excuses-only-its-own-pair",
        [h[4] for h, _e in excused] == ["yeban-render"]
        and [v[4] for v in violations] == ["yeban-dsp"]
        and not stale,
    ))
    _excused, _violations, stale = resolve_hits("yeban-mcp", [], EXCEPTIONS)
    structural.append(("stale-exception-is-detected", [e.dep for e in stale] == ["yeban-render"]))
    excused, violations, stale = resolve_hits("yeban-elsewhere", probe, EXCEPTIONS)
    structural.append((
        "exception-scoped-to-its-crate",
        not excused and [v[4] for v in violations] == ["yeban-render", "yeban-dsp"] and not stale,
    ))

    # 例外表合法性：仓库里真正那条必须合法；下面六种坏表必须**每一种**都被拒
    good = EXCEPTIONS[0]
    structural.append(("registry-real-entry-is-valid", not validate_exception_registry(EXCEPTIONS, scope_crate="yeban-mcp")))
    structural.append(("registry-rejects-wildcard-dep", bool(validate_exception_registry((dataclasses.replace(good, dep="yeban-*"),)))))
    structural.append(("registry-rejects-dep-not-forbidden", bool(validate_exception_registry((dataclasses.replace(good, dep="serde"),)))))
    structural.append(("registry-rejects-duplicate-pair", bool(validate_exception_registry((good, good)))))
    structural.append(("registry-rejects-empty-provenance", bool(validate_exception_registry((dataclasses.replace(good, provenance=()),)))))
    structural.append(("registry-rejects-empty-reason", bool(validate_exception_registry((dataclasses.replace(good, tolerated_because="  "),)))))
    structural.append(("registry-rejects-out-of-scope-crate", bool(validate_exception_registry((dataclasses.replace(good, crate="yeban-app"),), scope_crate="yeban-mcp"))))

    failures += [f"结构判据不成立: {name}" for name, ok in structural if not ok]

    if failures:
        print("[FAIL] --self-test 未通过:", file=sys.stderr)
        for line in failures:
            print("        " + line, file=sys.stderr)
        return 1
    print(
        f"[ok] mcp-dependency-direction --self-test: {len(cases)} 组注入/放行/盲区判据"
        f" + {len(structural)} 组例外表结构判据全部成立"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
