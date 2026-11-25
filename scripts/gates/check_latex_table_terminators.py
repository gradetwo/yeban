#!/usr/bin/env python3
"""LaTeX 表格行终止检查（longtable 行尾双反斜杠门）。

扫描 `docs/manual-latex` 下的 `.tex` 文件，对含未转义 `&` 的表格数据行，
若行尾是单个反斜杠 `\\`（而不是 longtable 需要的 `\\\\`）即报错并点名
`文件:行号`。

为什么需要这条门：

- 单反斜杠在 TeX 里是合法控制空白，花括号配对仍然成立，现有花括号检查
  抓不到它；
- 它不能终止表格行，会把下一行并进当前行/列，属于源级错误；
- 这种错误在逐章抽查时容易漏，只有整册扫描才能稳定拦住。

用法：

    python3 scripts/gates/check_latex_table_terminators.py
    python3 scripts/gates/check_latex_table_terminators.py --root /path/to/docs/manual-latex
    python3 scripts/gates/check_latex_table_terminators.py --self-test

退出码：

- 0：扫描范围内所有表格数据行都以 `\\\\` 终止；
- 1：发现单反斜杠行；
- 2：扫描范围无效（目录不存在或没有 `.tex` 文件）——响亮失败，不静默绿；
- `--self-test` 用临时正/负夹具自检，0 表示两条判据都符合预期。

注意：默认根是仓库的 `docs/manual-latex`。该目录尚未合入 main 时，
直接运行会以 2 退出；`--root` 可指向已合入/待验证的文档树。
"""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DEFAULT_ROOT = REPO / "docs" / "manual-latex"

SKIP_DIRS = {
    ".git",
    "target",
    "node_modules",
    "dist",
    ".worktrees",
    ".cargo-home",
    ".cache",
    "artifacts",
}


@dataclass
class ScanResult:
    files: int
    rows: int
    errors: list[str]
    scope_error: str | None = None


def strip_comment(line: str) -> str:
    """去掉未转义的 `%` 注释，保留行号位置所需的原始行内容。

    逐字符前进：反斜杠跳过下一个字符（这样转义的百分号不会被当成注释起点），
    遇到裸 `%` 即截断。LaTeX 行内注释不跨行，故只看当前行。
    """
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == "\\":
            i += 2
            continue
        if ch == "%":
            return line[:i]
        i += 1
    return line


def has_unescaped_ampersand(line: str) -> bool:
    """判断是否有未转义的 `&`（转义的 & 不算表格对齐符）。"""
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == "\\":
            i += 2
            continue
        if ch == "&":
            return True
        i += 1
    return False


def tex_files(root: Path) -> list[Path]:
    """遍历 root 下的 `.tex`，在进入大目录前剪枝。"""
    out: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".tex"):
                out.append(Path(dirpath) / name)
    return sorted(out)


def scan_root(root: Path) -> ScanResult:
    root = root.resolve()
    if not root.is_dir():
        return ScanResult(
            files=0,
            rows=0,
            errors=[],
            scope_error=f"扫描范围无效：{root} 不存在或不是目录；扫描范围为 0",
        )

    files = tex_files(root)
    if not files:
        return ScanResult(
            files=0,
            rows=0,
            errors=[],
            scope_error=f"扫描范围无效：{root} 下没有 .tex 文件；扫描范围为 0",
        )

    errors: list[str] = []
    rows = 0
    for path in files:
        rel = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            body = strip_comment(line).rstrip()
            if not has_unescaped_ampersand(body):
                continue
            rows += 1
            if body.endswith("\\\\"):
                continue
            if body.endswith("\\"):
                snippet = line.strip()
                if len(snippet) > 180:
                    snippet = snippet[:177] + "..."
                errors.append(
                    f"{rel}:{lineno}: 表格数据行以单个反斜杠结尾"
                    f"（longtable 需要 \\\\）：{snippet}"
                )

    return ScanResult(files=len(files), rows=rows, errors=errors)


def self_test() -> int:
    """用临时目录构造正/负夹具，验证抽取器与红对照。"""
    with tempfile.TemporaryDirectory(prefix="latex-table-gate-") as tmp:
        base = Path(tmp)
        good_root = base / "good"
        good_root.mkdir()
        (good_root / "good.tex").write_text(
            r"""\begin{longtable}{ll}
a & b \\
c & d \\ % 行尾注释不影响判定
\end{longtable}
""",
            encoding="utf-8",
        )

        bad_root = base / "bad"
        bad_root.mkdir()
        (bad_root / "bad.tex").write_text(
            r"""\begin{longtable}{ll}
a & b \
\end{longtable}
""",
            encoding="utf-8",
        )

        good_result = scan_root(good_root)
        bad_result = scan_root(bad_root)
        missing_result = scan_root(base / "missing")

        good_ok = (
            good_result.scope_error is None
            and good_result.files == 1
            and good_result.rows == 2
            and not good_result.errors
        )
        bad_ok = (
            bad_result.scope_error is None
            and bad_result.files == 1
            and bad_result.rows == 1
            and len(bad_result.errors) == 1
            and "bad.tex:2" in bad_result.errors[0]
        )
        missing_ok = missing_result.scope_error is not None

        if not (good_ok and bad_ok and missing_ok):
            print("self-test FAILED", file=sys.stderr)
            print(f"  good: {good_result}", file=sys.stderr)
            print(f"  bad:  {bad_result}", file=sys.stderr)
            print(f"  missing: {missing_result}", file=sys.stderr)
            return 1

        print("self-test ok: 正夹具 0 红、负夹具点名 bad.tex:2、缺目录响亮失败")
        return 0

def main() -> int:
    parser = argparse.ArgumentParser(description="检查 LaTeX 表格数据行是否以双反斜杠终止")
    parser.add_argument(
        "--root",
        type=Path,
        default=DEFAULT_ROOT,
        help="要扫描的文档根（默认仓库 docs/manual-latex）",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="用临时正/负夹具自检，不扫描真实文档",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    result = scan_root(args.root)
    if result.scope_error:
        print(result.scope_error, file=sys.stderr)
        return 2

    print(f"扫描 {result.files} 个 .tex 文件，{result.rows} 个表格数据行")
    if result.errors:
        print(f"\nLaTeX 表格行终止检查未通过（{len(result.errors)} 行）：", file=sys.stderr)
        for error in result.errors:
            print(f"  - {error}", file=sys.stderr)
        return 1

    print("LaTeX 表格行终止检查通过（所有含 & 的数据行均以 \\\\ 终止）。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
