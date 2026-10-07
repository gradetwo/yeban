#!/usr/bin/env python3
"""Linux Tier-1 golden 图集清单 (`tests/golden/linux/MANIFEST.txt`) 的机械判据。

为什么需要这条门禁: 清单最后那张表的 5 行 `filename / sha256 / bytes` 是**手工抄写**的 ——
仓库里没有任何生成器 (`grep MANIFEST scripts/` 找不到写它的东西), 而它承载的是「磁盘上这 5 张
PNG 到底是哪 5 张」的唯一记录。手工转录就会漂移, 而且漂移**不会让任何 Rust 测试变红**:
`assert_matches_golden` 只读 PNG 本身, 从不读 MANIFEST (`src/test_port_adapter.rs`)。
所以漂移只有靠一条专门核对它的门禁才拦得住。

判据 (每条都是硬失败 —— 缺表、畸形表、空表一律**不许**当成通过):
  1. 表头必须是字面的 `文件⇥sha256⇥字节` (TAB 分隔), 且在整份文件里恰好出现一次;
  2. 表必须落在文件的**末尾**: 表头之后只剩这一张表, 不许再有空行或别的正文;
  3. 每行必须是 3 个 TAB 分隔字段: 裸文件名 / 64 位**小写**十六进制 sha256 / 十进制字节数;
  4. 每个字段都要对**磁盘上的真实字节**核验: 文件存在、sha256 相符、字节数相符;
  5. 本目录里除 `MANIFEST.txt` 之外的**每个文件都必须被表列出** (不许有未登记的文件);
  6. 同一个文件名不许出现两次 (重复行 = 有人在拼接清单);
  7. 表前必须留有这份清单自身的**出处与复核区块** (生成地点 / 生成方式 / run id / 生成时间 /
     入库时间 / 为什么 / 复核 / 判据口径) —— 少任何一项都说明清单被削过, 出处不再可追。

⚠ 字节数**几乎不含信息**: 本仓 PNG 写入器是存储式 deflate, 1920×1080 恒为 6,222,418 字节
(`AGENTS.md` §6.3 规则 3)。区分内容的只有 sha256 —— 本脚本两个都核, 但别把"字节数相等"
读成"内容相同"。

指标 (先说单位再跑): "核对了 N 行" 数的是**清单表里的数据行数** (对象: 表里的行; 单位: 行);
每行贡献 **1 个 sha256 比对** + **1 个字节数比对**。成功行把这三个数都打印出来, 于是读者
能看见它真的量了东西, 而不是对着一张空表点头。

用法:
    python3 scripts/gates/check_golden_manifest.py                # 只判 (退出码 0/1)
    python3 scripts/gates/check_golden_manifest.py --write-table  # 显式选入: 只按磁盘重写表,
                                                                  # 人手写的表前区块逐字节不动

退出码: 0 = 清单与磁盘逐项一致; 1 = 任一判据不成立 (含表缺失 / 畸形 / 空表 / 未登记文件)。
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GOLDEN_DIR = REPO / "crates" / "yeban-app" / "tests" / "golden" / "linux"
MANIFEST = GOLDEN_DIR / "MANIFEST.txt"

#: 文档化的表头: 三个字段用 **TAB** 分隔 (注意不是空格 —— 空格分隔的"表头"是格式漂移)。
TABLE_HEADER = "文件\tsha256\t字节"

SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
#: 十进制且**不许有前导零** (`06222418` 是转录歧义, 不是字节数)。
BYTES_RE = re.compile(r"^(0|[1-9][0-9]*)$")

#: 表前必须出现的出处/复核锚点, 取自 `docs/adr/ADR-0003-save-button-top-bar-slot.md`
#: 的"批准后的再生成程序"与被手工写入的清单正文本身。
PROVENANCE = (
    ("生成地点", "生成地点"),
    ("生成方式", "生成方式"),
    ("run id", "run id (哪一次 CI run 产出)"),
    ("生成时间", "生成时间"),
    ("入库时间", "入库时间"),
    ("为什么", "为什么再生成"),
    ("复核", "入库前的复核记录"),
    ("判据口径", "判据口径"),
)


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def sha256_of(path: Path) -> str:
    """按 1 MiB 分块算文件的 sha256 (5 张各 ~6 MB, 不整份读进内存)。"""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


@dataclass
class Row:
    """表里的一行。`lineno` 是 1 基行号, `raw_*` 保留原文以便把两侧值都打出来。"""

    lineno: int
    name: str
    raw_sha: str
    raw_bytes: str
    sha: str | None = None
    size: int | None = None


@dataclass
class Parse:
    lines: list[str] = field(default_factory=list)
    header_index: int | None = None
    rows: list[Row] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)


def parse_manifest(text: str) -> Parse:
    """只做**结构**解析, 不碰磁盘。任何缺失/畸形都记进 `errors`, 绝不静默跳过。"""
    result = Parse(lines=text.splitlines())
    lines = result.lines

    exact = [index for index, line in enumerate(lines) if line == TABLE_HEADER]
    if not exact:
        # 失败时尽量指出"像表头但不是表头"的那一行, 让人一眼看到是 TAB / 字段名漂移。
        near = [
            index
            for index, line in enumerate(lines)
            if "sha256" in line and "字节" in line
        ]
        if near:
            index = near[0]
            result.errors.append(
                f"{rel(MANIFEST)}:{index + 1} 表头格式不符: 需要字面的 "
                f"`文件\\tsha256\\t字节` (TAB 分隔), 实到: {lines[index]!r}"
            )
        else:
            result.errors.append(
                f"{rel(MANIFEST)} 找不到表头行 —— 需要字面的 "
                f"`文件\\tsha256\\t字节` (TAB 分隔); 清单可能被截断或表被删掉"
            )
        return result

    if len(exact) > 1:
        result.errors.append(
            f"{rel(MANIFEST)}:{exact[1] + 1} 表头出现第 {len(exact)} 次 "
            f"(首次在 :{exact[0] + 1}) —— 清单里只允许一张表"
        )
        return result

    header = exact[0]
    result.header_index = header

    # ---- 判据 7: 表前必须留有出处与复核区块 ----
    prefix = "\n".join(lines[:header])
    for needle, label in PROVENANCE:
        if needle not in prefix:
            result.errors.append(
                f"{rel(MANIFEST)}:1-{header} 表前缺少清单出处/复核锚点 `{needle}` ({label}) "
                f"—— 这份清单被削过, 出处不可追"
            )

    # ---- 判据 2 + 3: 表是最后一块, 且每行形如 name⇥sha256⇥bytes ----
    body = lines[header + 1 :]
    if not body:
        result.errors.append(
            f"{rel(MANIFEST)}:{header + 1} 表是空的 (0 行) —— 空表不算通过, 判据会空转"
        )
        return result

    for offset, line in enumerate(body):
        lineno = header + 1 + offset + 1
        if line == "":
            result.errors.append(
                f"{rel(MANIFEST)}:{lineno} 表头之后出现空行 —— 表必须是文件的最后一块, "
                f"且行间不许有空行"
            )
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            result.errors.append(
                f"{rel(MANIFEST)}:{lineno} 行格式不符: 需要 3 个 TAB 分隔字段 "
                f"(文件名\\tsha256\\t字节), 实到 {len(fields)} 个字段"
            )
            continue
        name, raw_sha, raw_bytes = fields
        if not name or name in {".", ".."} or "/" in name or "\\" in name:
            result.errors.append(
                f"{rel(MANIFEST)}:{lineno} 文件名不是本目录下的裸文件名: {name!r}"
            )
            continue
        if not SHA256_RE.match(raw_sha):
            result.errors.append(
                f"{rel(MANIFEST)}:{lineno} {name} 的 sha256 不是 64 位小写十六进制: "
                f"{raw_sha!r}"
            )
            continue
        if not BYTES_RE.match(raw_bytes):
            result.errors.append(
                f"{rel(MANIFEST)}:{lineno} {name} 的字节数不是无前导零的十进制整数: "
                f"{raw_bytes!r}"
            )
            continue
        result.rows.append(
            Row(lineno=lineno, name=name, raw_sha=raw_sha, raw_bytes=raw_bytes)
        )

    # ---- 判据 6: 重复行 ----
    seen: dict[str, int] = {}
    for row in result.rows:
        if row.name in seen:
            result.errors.append(
                f"{rel(MANIFEST)}:{row.lineno} 文件名 {row.name} 与 :{seen[row.name]} "
                f"重复 —— 同一份表里一个文件只允许一行"
            )
        else:
            seen[row.name] = row.lineno

    return result


def verify() -> int:
    """完整判据。返回退出码; 失败细节逐条以 `[FAIL] file:line` 打到 stderr。"""
    if not GOLDEN_DIR.is_dir():
        print(f"[FAIL] 缺少 golden 目录: {rel(GOLDEN_DIR)}", file=sys.stderr)
        return 1
    if not MANIFEST.is_file():
        print(f"[FAIL] 缺少清单: {rel(MANIFEST)}", file=sys.stderr)
        return 1

    parsed = parse_manifest(MANIFEST.read_text(encoding="utf-8"))
    errors = list(parsed.errors)

    # ---- 判据 4: 每一行都对磁盘上的真实字节核验 ----
    checked: list[Row] = []
    for row in parsed.rows:
        path = GOLDEN_DIR / row.name
        if not path.is_file():
            errors.append(
                f"{rel(MANIFEST)}:{row.lineno} 行 {row.name} 列在表里, 但 "
                f"{rel(path)} 不存在 —— 表的行必须逐行对应磁盘上的真实文件"
            )
            continue
        real_sha = sha256_of(path)
        real_size = path.stat().st_size
        row.sha = real_sha
        row.size = real_size
        if real_sha != row.raw_sha:
            errors.append(
                f"{rel(MANIFEST)}:{row.lineno} {row.name} 的 sha256 不符: "
                f"表 {row.raw_sha} / 磁盘 {real_sha}"
            )
        if real_size != int(row.raw_bytes):
            errors.append(
                f"{rel(MANIFEST)}:{row.lineno} {row.name} 的字节数不符: "
                f"表 {row.raw_bytes} / 磁盘 {real_size}"
            )
        checked.append(row)

    # ---- 判据 5: 目录里不许有未登记的文件 ----
    listed = {row.name for row in parsed.rows}
    on_disk = sorted(
        path.name for path in GOLDEN_DIR.iterdir() if path.is_file() and path.name != MANIFEST.name
    )
    for name in on_disk:
        if name not in listed:
            errors.append(
                f"{rel(GOLDEN_DIR / name)} 在目录里但清单的表没有列出它 —— "
                f"表必须覆盖本目录的每一个文件"
            )

    if errors:
        print(f"[FAIL] golden-manifest(linux): {len(errors)} 条判据不成立:", file=sys.stderr)
        for message in errors:
            print(f"[FAIL] {message}", file=sys.stderr)
        return 1

    # ---- 成功: 打印指标, 让人看见真的量了东西 ----
    total = sum(row.size or 0 for row in checked)
    print(
        f"[ok] golden-manifest(linux): 核对了 {len(checked)} 行——"
        f"{len(checked)} 个 sha256 + {len(checked)} 个字节数"
        f"(表内 {len(checked)} 个文件全部与磁盘一致; 目录内无未登记文件; "
        f"表内字节合计 {total})"
    )
    return 0


def render_table() -> tuple[str, list[tuple[str, str, int]]]:
    """按磁盘事实渲染表 (文件名排序 ⇒ 输出确定)。"""
    entries: list[tuple[str, str, int]] = []
    for path in sorted(GOLDEN_DIR.iterdir(), key=lambda item: item.name):
        if not path.is_file() or path.name == MANIFEST.name:
            continue
        entries.append((path.name, sha256_of(path), path.stat().st_size))
    lines = [TABLE_HEADER]
    for name, digest, size in entries:
        lines.append(f"{name}\t{digest}\t{size}")
    return "\n".join(lines) + "\n", entries


def write_table() -> int:
    """显式选入: 只按磁盘重写表, 表前人手写的区块逐字节保留, 写完立刻重判。"""
    if not GOLDEN_DIR.is_dir():
        print(f"[FAIL] 缺少 golden 目录: {rel(GOLDEN_DIR)}", file=sys.stderr)
        return 1
    if not MANIFEST.is_file():
        print(f"[FAIL] 缺少清单: {rel(MANIFEST)}", file=sys.stderr)
        return 1

    old_text = MANIFEST.read_text(encoding="utf-8")
    lines = old_text.splitlines(keepends=True)
    flat = old_text.splitlines()
    exact = [index for index, line in enumerate(flat) if line == TABLE_HEADER]
    if len(exact) != 1:
        print(
            f"[FAIL] 拒绝重写: 表头 `文件\\tsha256\\t字节` 在 {rel(MANIFEST)} 里出现 "
            f"{len(exact)} 次 (必须恰好 1 次) —— 找不到唯一要替换的区块",
            file=sys.stderr,
        )
        return 1

    table, entries = render_table()
    if not entries:
        print(
            f"[FAIL] 拒绝重写: {rel(GOLDEN_DIR)} 里没有任何文件, 写出去会是一张空表",
            file=sys.stderr,
        )
        return 1

    header = exact[0]
    prefix = "".join(lines[:header])
    fresh = prefix + table

    if fresh == old_text:
        print(
            f"[ok] golden-manifest(linux) --write-table: 表已是磁盘事实, "
            f"{rel(MANIFEST)} 逐字节未改"
        )
        return 0

    # 说清楚改了什么: 每个文件的新旧 (sha256, 字节数)。
    before = {
        row.name: (row.raw_sha, row.raw_bytes)
        for row in parse_manifest(old_text).rows
    }
    after = {name: (digest, str(size)) for name, digest, size in entries}
    for name in sorted(set(before) | set(after)):
        old = before.get(name)
        new = after.get(name)
        if old == new:
            continue
        if old is None:
            print(f"  + 新增 {name}: sha256={new[0]} 字节={new[1]}")  # type: ignore[index]
        elif new is None:
            print(f"  - 删去 {name}: 表里原有 sha256={old[0]} 字节={old[1]}, 磁盘上已无此文件")
        else:
            print(
                f"  ~ 更新 {name}: sha256 {old[0]} -> {new[0]} (字节 {old[1]} -> {new[1]})"
            )
    MANIFEST.write_text(fresh, encoding="utf-8")
    print(
        f"[ok] golden-manifest(linux) --write-table: 按磁盘重写了 {len(entries)} 行, "
        f"表前人手写的区块逐字节保留; 现在重判一次"
    )
    return verify()


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Linux Tier-1 golden 清单 (MANIFEST.txt) 的机械判据"
    )
    parser.add_argument(
        "--write-table",
        action="store_true",
        help="显式选入: 只按磁盘重写清单末尾的表, 表前人手写的区块不动",
    )
    args = parser.parse_args()
    if args.write_table:
        return write_table()
    return verify()


if __name__ == "__main__":
    raise SystemExit(main())
