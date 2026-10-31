#!/usr/bin/env python3
"""集成测试文件「零判据」守卫。

## 为什么要有它

`crates/*/tests/*.rs` 的每个文件都是一个会被 `cargo test` 执行的二进制。
**一个没有任何 `#[test]` 的文件照样编译、照样报绿**（0 个测试 = `test result: ok. 0 passed`），
于是「新增了一个测试文件」与「新增了一条判据」在门禁读数上**无法区分**。
本守卫把这条缺口机械化：`tests/*.rs` 里零 `#[test]` 的文件必须**逐条登记**为具名例外（带理由）。

## 三个方向都要挡

1. **新增**零判据文件而未登记 ⇒ 红；
2. **登记过的例外变了性质**（现在有 `#[test]` 了、或包装的源码没了判据）⇒ 红（陈旧条目）；
3. **语料缩水**（有人把 `tests/` 整批挪走 / 改成非 `*.rs`）⇒ 红（`MIN_FILES` 下界）。

## 例外的两种合法形态（都能被**机检**，不是只写在理由里）

* `path_wrapper`：文件本身没有 `#[test]`，用 `#[path = "..."]` 引入**别处**的判据源码。
  机检：`#[path]` 目标必须存在，且目标（或其 `#[path]` 链）里真的有 `#[test]`。
* `harness_false`：`Cargo.toml` 里 `[[test]]` 声明了 `harness = false` 的运行期腿
  （计数型全局分配器 / 实时线程契约）。它们自己 `fn main()`，由 libtest 之外的方式报错。
  机检：同 crate 的 `Cargo.toml` 里该 `[[test]]` 段必须仍写着 `harness = false`。

退出码：0 通过；1 红（含陈旧条目）。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# 被扫目录：`crates/*/tests` 的**顶层** `*.rs`（与 cargo 的自动发现同口径）。
TESTS_GLOB = "crates/*/tests/*.rs"

# 下界＝当前实测值（R93 / R115 同口径：删文件是单向偷工，只有下界能挡）。
MIN_FILES = 293
MIN_FILES_WITH_CRITERIA = 286

# 已评审例外。`kind` 决定哪条机检生效；理由必须说清"为什么它没有 #[test] 仍然在测东西"。
EXCEPTIONS: list[dict[str, str]] = [
    {
        "path": "crates/yeban-app/tests/real_ui_tier1.rs",
        "kind": "path_wrapper",
        "reason": "用 #[path] 原样引入 src/test_port_adapter.rs 的判据（第二个 cargo 目标，"
        "让该批判据真的被执行）；判据本体不在本文件。",
    },
    {
        "path": "crates/yeban-engine/tests/meter_rt_contract.rs",
        "kind": "harness_false",
        "reason": "harness = false 的实时线程契约腿：自己开线程/跑音频回调并自检。",
    },
    {
        "path": "crates/yeban-engine/tests/snapshot_retire_churn.rs",
        "kind": "harness_false",
        "reason": "harness = false：快照退役回收的计数型腿，libtest 自身会在进程里分配，"
        "会污染计数。",
    },
    {
        "path": "crates/yeban-engine/tests/synth_rt_zero_alloc.rs",
        "kind": "harness_false",
        "reason": "harness = false：合成器回调零分配的计数腿（同上，libtest 会污染分配计数）。",
    },
    {
        "path": "crates/yeban-engine/tests/transport_rt_zero_alloc.rs",
        "kind": "harness_false",
        "reason": "harness = false：走带线程零分配的计数腿（同上）。",
    },
    {
        "path": "crates/yeban-render/tests/pure_modules_scaffold.rs",
        "kind": "path_wrapper",
        "reason": "用 #[path] 把 verify/pure_modules.rs 的 115 条脚手架判据接进 cargo 门禁。",
    },
    {
        "path": "crates/yeban-render/tests/support_scaffolds.rs",
        "kind": "path_wrapper",
        "reason": "用 #[path] 把 examples/support/ 下两份零依赖脚手架（38 条判据）接进门禁。",
    },
    {
        "path": "crates/yeban-mcp/tests/section_pure_scaffold.rs",
        "kind": "path_wrapper",
        "reason": "用 #[path] 把 verify/section_pure.rs 的开环判据（含未知 materialSource 的"
        "拒绝面，tests/ 下无等价物）接进 cargo 门禁；同一先例见 yeban-render 的上一条。",
    },
]

PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')
TEST_ATTR = "#[test]"


def tests_files() -> list[Path]:
    return sorted(REPO.glob(TESTS_GLOB))


def has_test_attr(path: Path) -> bool:
    return TEST_ATTR in path.read_text(encoding="utf-8", errors="replace")


def path_targets(path: Path) -> list[Path]:
    text = path.read_text(encoding="utf-8", errors="replace")
    return [(path.parent / target).resolve() for target in PATH_ATTR.findall(text)]


def wrapper_reaches_criteria(path: Path) -> tuple[bool, list[str]]:
    """沿 `#[path]` 链找 `#[test]`；返回 (找到了吗, 走过的路径)。"""
    seen: list[str] = []
    stack = [path]
    while stack:
        current = stack.pop()
        if not current.is_file():
            continue
        if str(current) in seen:
            continue
        seen.append(str(current))
        if has_test_attr(current):
            return True, seen
        stack.extend(path_targets(current))
    return False, seen


def harness_false_holds(path: Path) -> bool:
    """同 crate 的 Cargo.toml 里，该 [[test]] 段必须仍写着 harness = false。"""
    manifest = path.parent.parent / "Cargo.toml"
    if not manifest.is_file():
        return False
    stem = path.stem
    text = manifest.read_text(encoding="utf-8", errors="replace")
    # 按 [[test]] 切段，逐段找 name = "<stem>"。
    for block in re.split(r"^\[\[test\]\]", text, flags=re.MULTILINE)[1:]:
        block = block.split("\n[[", 1)[0]
        if re.search(rf'^\s*name\s*=\s*"{re.escape(stem)}"\s*$', block, re.MULTILINE):
            return re.search(r"^\s*harness\s*=\s*false\s*$", block, re.MULTILINE) is not None
    return False


def main() -> int:
    files = tests_files()
    with_criteria = [path for path in files if has_test_attr(path)]
    zero = [path for path in files if path not in with_criteria]

    failures: list[str] = []

    # ③ 语料下界：删文件不能让判据变绿。
    if len(files) < MIN_FILES:
        failures.append(
            f"被扫 tests/*.rs 只有 {len(files)} 个 < 下界 {MIN_FILES}"
            "（删/挪测试文件是单向偷工）"
        )
    if len(with_criteria) < MIN_FILES_WITH_CRITERIA:
        failures.append(
            f"含 #[test] 的文件只有 {len(with_criteria)} 个 < 下界 {MIN_FILES_WITH_CRITERIA}"
        )

    registered = {REPO / entry["path"]: entry for entry in EXCEPTIONS}

    # ① 新增零判据文件未登记。
    for path in zero:
        if path not in registered:
            failures.append(f"零判据文件未登记：{path.relative_to(REPO)}")

    # ② 登记条目必须仍然是它自称的形态。
    for path, entry in registered.items():
        rel = path.relative_to(REPO)
        if not path.is_file():
            failures.append(f"例外条目陈旧（文件不存在）：{rel}")
            continue
        if has_test_attr(path):
            failures.append(f"例外条目陈旧（现在有 #[test] 了，应删除登记）：{rel}")
            continue
        if entry["kind"] == "path_wrapper":
            found, seen = wrapper_reaches_criteria(path)
            if not found:
                failures.append(
                    f"例外条目失效：{rel} 的 #[path] 链里没有任何 #[test]（走过 {len(seen)} 个文件）"
                )
        elif entry["kind"] == "harness_false":
            if not harness_false_holds(path):
                failures.append(
                    f"例外条目失效：{rel} 的 [[test]] 段不再写着 harness = false"
                )
        else:
            failures.append(f"未知的例外 kind={entry['kind']!r}：{rel}")

    print(
        f"[check-test-criteria] 扫过 {len(files)} 个 tests/*.rs；"
        f"含判据 {len(with_criteria)} 个；零判据 {len(zero)} 个；"
        f"登记例外 {len(EXCEPTIONS)} 条"
    )
    if failures:
        for failure in failures:
            print(f"[FAIL] {failure}", file=sys.stderr)
        return 1
    print("[ok] 每个 tests/*.rs 要么自带判据，要么是已登记且可机检的例外")
    return 0


if __name__ == "__main__":
    sys.exit(main())
