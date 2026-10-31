#!/usr/bin/env python3
"""端到端 verify 脚本的**防掏空**守卫 [MCP-TOOL-005]。

## 为什么存在

`crates/yeban-mcp/verify/e2e_audio_clip_stdio.sh` 与 `idempotency_replay_schema.py`
是 2026-10-31（台账 §22）才从"只打印"改成"断言 + 非零退出"的，执行者是 `ci.yml` 的
`e2e (MCP stdio 复跑 / 幂等重放契约)` 那条腿。**但那条腿只保证脚本跑到退出码 0**：

- 若有人把末尾的 `sys.exit(1)` 删掉 ⇒ 脚本永远 0 退出，e2e 腿**静默变绿**；
- 若有人把某条 `check(...)` 换成一行打印 ⇒ 判据消失，CI 依旧绿。

这正是本仓库刚刚修过两遍的那一族缺陷（§21.1 的 `audio_pins.py`、§22.1 的 stdio 脚本）。
本守卫把"**判据还在、而且真的有牙**"钉成机械读数。

## 口径（⛔ 宽口径已被证明会放过东西）

`§0.6.0p` 候选 B 的教训：⛔ 不要写"文件里有 assert 字样即算过"。本守卫逐项点名，
下界全部是**现读的具名数字**（2026-10-31，线 tip `f0ba67de`）：

- `check(` **调用点** ≥ 7（数法：`check(` 出现次数减去 `def check(` 定义那一处；
  7 个调用点里 2 个在逐渲染循环内 ⇒ 一次复跑**执行** 9 条判据 —— 实测见 run `41203744827`）；
- 7 个判据**名字**逐名在册（循环里那两条按后缀匹配）；
- stdio 脚本必须保留**非零退出路径**（`sys.exit(1)`）；
- 两份脚本都必须继续给 `yeban_import_audio` 传 **`license`** —— 它是必填参数，
  不传时夹具会在工具边界被拒（`licenseRequired`），于是判的红变成"夹具不合法"而不是它名义上判的东西。

守卫**只读文本**，不执行被检脚本（执行是 e2e 腿的活）；文件缺失同样报红（⛔ 防空转）。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
STDIO = REPO / "crates/yeban-mcp/verify/e2e_audio_clip_stdio.sh"
REPLAY = REPO / "crates/yeban-mcp/verify/idempotency_replay_schema.py"

MIN_CHECK_CALL_SITES = 7
CRITERIA_NAMES = (
    "responses_are_complete_and_error_free",
    "import_registers_and_places_the_clip",
    "save_persists_the_asset",
    "receipt_matches_the_file",
    "every_asset_has_bytes_and_frames",
    "no_asset_reported_as_missing_bytes",
    "two_renders_are_byte_identical",
)


def fail(problems: list[str], message: str) -> None:
    problems.append(message)


def has_license_key(text: str) -> bool:
    """只认**参数键**形态的 `"license":`。

    ⚠ 第一版写的是 `"license" not in text`，实测**漏判**：把两份脚本里的
    `"license": "CC0-1.0"` 全删掉，散文注释里的 "license" 一词仍让守卫算过 ⇒ 守卫自己
    犯了它要防的错（宽口径）。参数键才是"夹具会否被 `licenseRequired` 拒"的真读数。
    """
    return re.search(r'"license"\s*:', text) is not None


def check_stdio(problems: list[str]) -> list[str]:
    """返回 stdio 脚本的读数行（同时把不合规项写进 `problems`）。"""
    if not STDIO.is_file():
        fail(problems, f"缺少被检脚本 {STDIO}")
        return []
    text = STDIO.read_text(encoding="utf-8")

    # 行首锚定：⛔ 用 `\bcheck\(` —— 脚本头注释里那句"`check(...)` 调用点"会被它数进去
    # （实测第一版因此报 8，真值是 7：注释一行 = 一次假调用点）。
    call_sites = len(re.findall(r"(?m)^[ \t]*check\(", text))
    if call_sites < MIN_CHECK_CALL_SITES:
        fail(
            problems,
            f"`check(` 调用点只剩 {call_sites} 个 (下界 {MIN_CHECK_CALL_SITES}) "
            "⇒ 判据被删或被打字取代",
        )
    for name in CRITERIA_NAMES:
        if name not in text:
            fail(problems, f"判据 `{name}` 不在 stdio 脚本里 —— 名字漂移或被掏空")
    if "sys.exit(1)" not in text:
        fail(problems, "stdio 脚本没有 `sys.exit(1)`：任一判据为红也会 0 退出 ⇒ e2e 腿静默变绿")
    if not has_license_key(text):
        fail(
            problems,
            'stdio 脚本没有 `"license":` 参数键 ⇒ 夹具会在工具边界被 `licenseRequired` 拒',
        )
    return [
        f"stdio 判据：`check(` 调用点 {call_sites} (下界 {MIN_CHECK_CALL_SITES})、"
        f"具名判据 {len(CRITERIA_NAMES)} 条逐名在册、非零退出路径在场、`license` 在场"
    ]


def check_replay(problems: list[str]) -> list[str]:
    if not REPLAY.is_file():
        fail(problems, f"缺少被检脚本 {REPLAY}")
        return []
    text = REPLAY.read_text(encoding="utf-8")
    for needle, why in (
        ("return 1", "无 `return 1` ⇒ 重放形状漂移也会以 0 退出"),
        ("ReplayedToolResponse", "不再引用 `ReplayedToolResponse` ⇒ 本脚本名义上判的东西消失了"),
    ):
        if needle not in text:
            fail(problems, f"idempotency 脚本缺少 `{needle}`：{why}")
    if not has_license_key(text):
        fail(
            problems,
            'idempotency 脚本没有 `"license":` 参数键 ⇒ 两次调用都被工具边界拒绝，'
            "判的红将是\"夹具不合法\"而非\"重放形状漂移\"",
        )
    return ["idempotency 判据：`return 1` / `ReplayedToolResponse` 在场、`license` 参数键在场"]


def main(argv: list[str]) -> int:
    problems: list[str] = []
    readings = check_stdio(problems) + check_replay(problems)
    for line in readings:
        print(f"[读数] {line}")
    if problems:
        print()
        for problem in problems:
            print(f"[FAIL] {problem}")
        return 1
    print("[ok] 两份端到端 verify 脚本的判据与非零退出路径都在场")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
