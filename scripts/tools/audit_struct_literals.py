"""对账「具名字段的结构体字面量」—— 既报少插的键，也报多插／重复的键。

为什么需要它：给 `PlanOptions` 加一个字段后，任何 `PlanOptions { … }` 字面量少写这个键
就是 CI 的 `E0063`，多写就是 `E0560`。这类错本机编不了（`yeban-mcp` 的测试要整 crate 编，
那是 CI 的活），一轮 CI ≈ 1 小时 ⇒ 必须在推送前用文本对账量出来。

口径（⚠ 任何读数都要说明它排除了什么）：
- 只数**显式列键**的字面量；带 `..base`（结构更新语法）的字面量⛔ 报「缺键」，但⚠ 报「多键」。
- 排除 `//`／`///` 注释里的同名字串（那是文本，不是代码）；⛔ 不动字符串里的 `//`。
- 单位＝「每个字面量的顶层键名相对字段表的差集」，⛔ 不是「行数」。

用法：
    python3 scripts/tools/audit_struct_literals.py <Struct> <定义文件> <扫描目录或文件>…
    python3 scripts/tools/audit_struct_literals.py --self-test   # RC 必须为 0
"""

from __future__ import annotations

import pathlib
import re
import sys

BRACKETS_OPEN = "{[("
BRACKETS_CLOSE = "})]"
LITERAL = re.compile(r"\b([A-Z]\w*)\s*\{")
KEY = re.compile(r"^(\w+)\s*:")
SHORT = re.compile(r"^\w+$")


def strip_line_comments(text: str) -> str:
    """逐行去掉 `//` 之后的部分，但字符串内部的 `//` ⛔ 动。"""
    out = []
    for line in text.split("\n"):
        instr, esc, cut = False, False, None
        i = 0
        while i < len(line):
            ch = line[i]
            if instr:
                if esc:
                    esc = False
                elif ch == "\\":
                    esc = True
                elif ch == '"':
                    instr = False
            elif ch == '"':
                instr = True
            elif ch == "/" and line.startswith("//", i):
                cut = i
                break
            i += 1
        out.append(line if cut is None else line[:cut])
    return "\n".join(out)


def scan(text: str, start: int, until_close: bool):
    """自 `start` 起做「字符串感知的括号行走」。

    `until_close=True` ⇒ 返回 (最外层 `{` 的内部文本, 其闭括号位置)，不配对则 (None, len)。
    `until_close=False` ⇒ 只返回顶层逗号切出的片段列表。
    """
    depth = 0
    instr = False
    esc = False
    items: list[str] = []
    cur: list[str] = []
    body_start = None
    for j in range(start, len(text)):
        ch = text[j]
        if instr:
            cur.append(ch)
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                instr = False
            continue
        if ch == '"':
            instr = True
            cur.append(ch)
            continue
        if ch in BRACKETS_OPEN:
            depth += 1
            if until_close and depth == 1 and ch == "{":
                body_start = j + 1
                continue
        elif ch in BRACKETS_CLOSE:
            depth -= 1
            if until_close and depth == 0 and ch == "}":
                return text[body_start:j], j
        elif ch == "," and depth == 0 and not until_close:
            items.append("".join(cur))
            cur = []
            continue
        cur.append(ch)
    if until_close:
        return None, len(text)
    if cur:
        items.append("".join(cur))
    return items, len(text)


def literal_body(text: str, brace_at: int):
    return scan(text, brace_at, True)


def top_level_items(body: str):
    """按**顶层**逗号切 body。`(` 与 `)` 必须同时计数 —— 只数 `{[`（⛔ 漏 `)`）会让整个
    body 变成一项，于是"每个字面量都缺 12 个键"这种**假阳性**；`--self-test` 钉住这条。"""
    items, _ = scan(body, 0, False)
    return items


def field_table(def_text: str, struct: str):
    m = re.search(
        r"pub struct %s(?:<[^>]*>)?\s*\{(.*?)\n\}" % re.escape(struct), def_text, re.S
    )
    if not m:
        raise SystemExit("找不到 `pub struct %s` 的字段表" % struct)
    return re.findall(r"^\s*pub (\w+):", m.group(1), re.M)


def audit(struct: str, fields, text: str):
    """返回 [(行号, 缺键, 多余键, 重复键, 是否有 ..base)]。"""
    hits = []
    for m in LITERAL.finditer(text):
        if m.group(1) != struct:
            continue
        body, _ = literal_body(text, text.index("{", m.start()))
        if body is None:
            continue
        items = top_level_items(body)
        has_base = any(re.match(r"^\s*\.\.", it) for it in items)
        keys = []
        for it in items:
            it = it.strip()
            if has_base and it.startswith(".."):
                continue
            km = KEY.match(it)
            if km:
                keys.append(km.group(1))
            elif SHORT.match(it):
                keys.append(it)
        missing = [x for x in fields if x not in keys]
        extra = [x for x in keys if x not in fields]
        dup = sorted({x for x in keys if keys.count(x) > 1})
        if (missing and not has_base) or extra or dup:
            hits.append(
                (text[: m.start()].count("\n") + 1, missing, extra, dup, has_base)
            )
    return hits


def self_test() -> int:
    """正对照＝必须看得见缺陷；负对照＝必须看不见；另有括号配平与注释两条回归。"""
    fields = ["a", "b", "c"]
    fails = []

    def check(name, text, expect_hit):
        got = bool(audit("Cfg", fields, strip_line_comments(text)))
        if got != expect_hit:
            fails.append("%s：期望%s，实际相反" % (name, "报缺陷" if expect_hit else "干净"))

    check("负对照·列全键", "let x = Cfg {\n    a: 1,\n    b: 2, c: 3,\n};\n", False)
    check("正对照·少一键", "let x = Cfg { a: 1, b: 2 };\n", True)
    check("正对照·多一键", "let x = Cfg { a: 1, b: 2, c: 3, zz: 4 };\n", True)
    check("正对照·重复键", "let x = Cfg { a: 1, a: 2, b: 3, c: 4 };\n", True)
    check("`..base`·允许缺键", "let x = Cfg { ..base };\n", False)
    check("`..base`·仍报多键", "let x = Cfg { zz: 1, ..base };\n", True)
    check("注释里的同名字串", "// let x = Cfg { a: 1 };\nlet y = Cfg { a: 1, b: 2, c: 3 };\n", False)
    check("值里有嵌套字面量", 'let x = Cfg { a: Ok(Something { z: 1 }), b: 2, c: 3 };\n', False)

    body = ' a: Some("x"), b: None, c: Some("y"),'
    if len(top_level_items(body)) != 3:
        fails.append("顶层逗号切分失效：得到 %d 项（括号未配平）" % len(top_level_items(body)))

    for msg in fails:
        print("FAIL   %s" % msg)
    print("self-test: %d/%d 项对照通过" % (9 - len(fails), 9))
    return 1 if fails else 0


def main(argv) -> int:
    if "--self-test" in argv:
        return self_test()
    if len(argv) < 3:
        print(__doc__)
        return 2
    struct, def_file = argv[0], argv[1]
    fields = field_table(pathlib.Path(def_file).read_text(), struct)
    print("%s 字段表 (%d): %s" % (struct, len(fields), " ".join(fields)))
    bad = literals = 0
    for arg in argv[2:]:
        p = pathlib.Path(arg)
        files = sorted(p.glob("*.rs")) if p.is_dir() else [p]
        for f in files:
            text = strip_line_comments(f.read_text())
            literals += sum(1 for m in LITERAL.finditer(text) if m.group(1) == struct)
            for line, missing, extra, dup, has_base in audit(struct, fields, text):
                bad += 1
                print(
                    "BAD    %s:%d base=%s missing=%s extra=%s dup=%s"
                    % (f, line, has_base, missing, extra, dup)
                )
    print("扫描 %s 字面量 %d 处 / 缺陷 %d 处" % (struct, literals, bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
