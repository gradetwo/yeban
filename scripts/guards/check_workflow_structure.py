#!/usr/bin/env python3
"""R65 / ⭐ #172: 工作流结构自检 —— 工作流的改动必须至少被一条自动步骤验证。

动机 (R65): 一次只改 `.github/workflows/ci.yml` + 文档的推送会让 `plan` 判出空
crate 集合, 于是 `rust` 矩阵腿**整体 skipped**, 新加的腿在它自己的提交上一次都
没跑。因此本守卫**不依赖 crate 集合**: 只要 `checks` job 跑, 它就验证本文件可
解析、且关键腿与它们的守卫在位。

⭐ #172 起本守卫管的事 (取代 R64 的"恰好 2 条名字含 `导出特性` 的腿"):
  合并后的 app 特性腿只有 **一条 clippy + 一条 test**, 特性清单集中在 workflow 级
  `env.APP_FEATURES`。数"腿的条数"只能证明**形式在**, 不能证明**每个 feature 的
  判据都被编进来** —— 而合并恰恰把"少一个 feature"变成了静默事故: 腿还是一条,
  只是里面少了一个开关, 该 feature 的判据就**一条都不跑**。于是本守卫改判**语义**:

  ① `APP_FEATURES` 必须存在、非空、无重复;
  ② 它必须与 `crates/yeban-app/Cargo.toml` 的 `[features]` **机械一致**:
     - 声明了、却没进清单、且不在 `AUTO_CI_EXCLUDED`（带书面理由的豁免）里 ⇒ 红
       （⭐ 这就是"某个 feature 被从合并列表里删掉"的机械指纹）;
     - 清单里有、`Cargo.toml` 没声明 ⇒ 红（拼写错 / 改名残留）;
     - 豁免表里写着一个已不存在的特性 ⇒ 红（豁免自己会腐烂）;
     - 同一特性既在豁免里又在清单里 ⇒ 红（两种说法互相矛盾）。
  ③ 清单必须**真的被用上**: `rust` job 里至少有一条 `test` 腿与一条 `clippy` 腿,
     它们的 `run` 引用 `APP_FEATURES` 且带 `--features`; `test` 腿还必须带
     `--no-fail-fast`（失败可定位: 一个二进制红了不掩盖其余）。
  ④ R64 的意图（导出族必须真的编进来）由 ② 覆盖, 但仍**显式点名**, 防止有人把
     豁免表扩到这两个上而让 `logic.rs` / `als.rs` 退回"从不编译"。
  ⑤ windows 平台腿仍在（R60/R63 的验证面）。

本守卫刻意**不**钉"用哪个脚本过滤": 那是实现细节; 钉住的是"清单是单一事实源、
被 --features 真正使用、且与 Cargo.toml 不漂移"这三个性质。
"""

from __future__ import annotations

import pathlib
import sys
import tomllib

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
APP_MANIFEST = ROOT / "crates" / "yeban-app" / "Cargo.toml"

#: 特性清单的单一事实源（workflow 级 `env:` 的键名）。
FEATURE_ENV = "APP_FEATURES"

#: R64 点名要求真的被编进来的两个开关（该特性的代码在默认档下**从不编译**）。
R64_REQUIRED = ("experimental-als-export", "experimental-logic-export")

#: 明确**不**纳入自动档特性腿的 app 特性 + 书面理由。
#: ⚠ 加到这里的门槛是"该特性已经由**另一条**自动/手动门禁真的跑到", ⛔ 不是"这轮没空"。
AUTO_CI_EXCLUDED = {
    "ui-test-port": (
        "由 `[dev-dependencies] yeban-ui-test-port` + 自动发现的 `tests/real_ui_tier1.rs` "
        "在默认档执行（`cargo test --all-targets` 必然跑到），不需要 `--features`；"
        "见 `crates/yeban-app/Cargo.toml` 的 dev-dependencies 注释"
    ),
    "ui-mcp-http": (
        "会经 `yeban-ui-mcp` 把 GUI 控制面拖进每个测试配置（磁盘/时间成本大）；"
        "它由手动档 `gates-manual.yml` 的 `all-features` 档位编译并测试"
    ),
}


def fail(msg: str) -> None:
    print(f"[FAIL] {msg}")
    sys.exit(1)


def declared_app_features() -> set[str]:
    """app 的 `[features]` 键（去掉 `default` —— 它不是"开关"，而是默认集合）。"""
    data = tomllib.loads(APP_MANIFEST.read_text(encoding="utf-8"))
    features = data.get("features") or {}
    if not isinstance(features, dict) or not features:
        fail(f"{APP_MANIFEST} 没有 `[features]` 表 ⇒ 本判据空转，不能算证据")
    return {name for name in features if name != "default"}


def workflow_feature_list(doc: dict) -> list[str]:
    env = doc.get("env") or {}
    if not isinstance(env, dict):
        fail("ci.yml 顶层 `env:` 不是映射")
    raw = env.get(FEATURE_ENV)
    if not isinstance(raw, str) or not raw.strip():
        fail(
            f"ci.yml 顶层 env 缺少 `{FEATURE_ENV}`（或其值为空）—— "
            "合并后的特性腿必须在**一个**地方声明清单，⛔ 不要在各腿里各写一份"
        )
    names = [part.strip() for part in raw.split(",")]
    if any(not name for name in names):
        fail(f"`{FEATURE_ENV}` 里有空项（多余的逗号？）: {raw!r}")
    if len(names) != len(set(names)):
        dupes = sorted({n for n in names if names.count(n) > 1})
        fail(f"`{FEATURE_ENV}` 里有重复项: {dupes}")
    return names


def step_text(step: dict) -> str:
    chunks: list[str] = []
    run = step.get("run")
    if isinstance(run, str):
        chunks.append(run)
    env = step.get("env")
    if isinstance(env, dict):
        chunks.extend(str(value) for value in env.values())
    return "\n".join(chunks)


def main() -> int:
    try:
        doc = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001
        fail(f"{WORKFLOW} 无法解析: {exc}")

    if not isinstance(doc, dict):
        fail(f"{WORKFLOW} 顶层不是映射")
    jobs = doc.get("jobs") or {}
    if not jobs:
        fail("ci.yml 没有任何 job")

    for name, job in jobs.items():
        if not isinstance(job, dict):
            fail(f"job {name} 不是映射")
        # uses-reusable / 只有 runs-on 的占位 job 也要求 steps 非空。
        if not job.get("runs-on"):
            fail(f"job {name} 缺少 runs-on")
        if not job.get("steps"):
            fail(f"job {name} 缺少 steps")

    # ---- ① 清单本身 -------------------------------------------------------
    enabled = workflow_feature_list(doc)
    enabled_set = set(enabled)

    # ---- ② 清单 ↔ Cargo.toml ---------------------------------------------
    declared = declared_app_features()
    excluded = set(AUTO_CI_EXCLUDED)

    missing = sorted(declared - enabled_set - excluded)
    if missing:
        fail(
            "这些 feature 在 `crates/yeban-app/Cargo.toml` 里声明了, 却既不在 "
            f"`{FEATURE_ENV}` 里、也不在 `AUTO_CI_EXCLUDED` 的书面豁免里: {missing}\n"
            "  ⇒ 合并后它们的判据会**一条都不跑**（这正是 #172 要防的事故）。\n"
            "  ⇒ 要么加进清单, 要么在 AUTO_CI_EXCLUDED 里写清它由哪条门禁覆盖。"
        )

    unknown = sorted(enabled_set - declared)
    if unknown:
        fail(
            f"`{FEATURE_ENV}` 里的这些名字 `crates/yeban-app/Cargo.toml` 没有声明: {unknown}\n"
            "  ⇒ merged 腿对任何 crate 传不存在的 `--features` 都会直接报错。"
        )

    contradictory = sorted(enabled_set & excluded)
    if contradictory:
        fail(
            f"这些 feature 同时出现在 `{FEATURE_ENV}` 与 `AUTO_CI_EXCLUDED` 里: {contradictory}\n"
            "  ⇒ 要么测它、要么豁免它, ⛔ 不要两种说法并存。"
        )

    rotten = sorted(excluded - declared)
    if rotten:
        fail(
            f"`AUTO_CI_EXCLUDED` 里的这些名字已经不是 app 声明的 feature 了: {rotten}\n"
            "  ⇒ 豁免表腐烂了; 删掉对应条目（否则它会为下一次改名背书）。"
        )

    # ---- ④ R64: 导出族必须真的被编进来 -----------------------------------
    for needle in R64_REQUIRED:
        if needle not in enabled_set:
            fail(
                f"R64 要求 `{needle}` 必须真的编进自动门禁, 但它不在 `{FEATURE_ENV}` 里\n"
                "  ⇒ `yeban-render` 的 `logic.rs` / `als.rs` 会退回'从不编译'。"
            )

    # ---- ③ 清单必须真的被 clippy / test 腿用上 ---------------------------
    rust = jobs.get("rust")
    if not rust:
        fail("ci.yml 缺少 rust 矩阵 job")
    steps = rust.get("steps") or []

    def legs(prefix: str) -> list[dict]:
        return [
            s
            for s in steps
            if isinstance(s, dict)
            and str(s.get("name", "")).startswith(prefix)
            and FEATURE_ENV in step_text(s)
        ]

    test_legs = legs("test")
    if not test_legs:
        fail(
            f"`rust` job 里没有任何 `test` 腿引用 `{FEATURE_ENV}` ⇒ 清单是孤儿, "
            "合并后的特性判据一条都不会跑"
        )
    clippy_legs = legs("clippy")
    if not clippy_legs:
        fail(
            f"`rust` job 里没有任何 `clippy` 腿引用 `{FEATURE_ENV}` ⇒ "
            "feature 档的 `-D warnings` 没有自动步骤覆盖"
        )

    for step in test_legs + clippy_legs:
        run = str(step.get("run", ""))
        if "--features" not in run:
            fail(
                f"腿 {step.get('name')!r} 引用了 `{FEATURE_ENV}` 却**没有**把它交给 "
                "`--features` ⇒ 清单读了但没用"
            )
    for step in test_legs:
        if "--no-fail-fast" not in str(step.get("run", "")):
            fail(
                f"腿 {step.get('name')!r} 缺少 `--no-fail-fast` ⇒ 合并后一个测试二进制红"
                "会让其余二进制不再执行, 失败无法定位（#172 的判据 4）"
            )

    # ---- ⑤ 平台腿 ---------------------------------------------------------
    windows = jobs.get("windows")
    if not windows:
        fail("ci.yml 缺少 windows 平台腿 (R60/R63 的验证面)")
    if not windows.get("steps"):
        fail("windows 腿缺少 steps")

    print(
        f"[ok] ci.yml 结构自检通过: {len(jobs)} 个 job, "
        f"rust job {len(steps)} 个 step（合并特性 test 腿 {len(test_legs)} 条 / "
        f"clippy 腿 {len(clippy_legs)} 条）, "
        f"{FEATURE_ENV} {len(enabled)} 条 / app 声明 {len(declared)} 条 / "
        f"书面豁免 {len(excluded)} 条, windows 腿 {len(windows['steps'])} 个 step"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
