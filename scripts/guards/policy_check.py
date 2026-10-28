#!/usr/bin/env python3
"""夜半 (Yeban) 机械红线守卫 (Mechanical redline guards).

把 AGENTS.md §2 的「绝对禁止项」与架构规范里的机械可判定条款变成**能变红的检查**。
每一条守卫都对应一个规范 ID；没有规范依据的检查不写进这里。

设计纪律 (docs/skills/yeban-dev-workflow/SKILL.md 规则 2):
    判据必须先能失败。本脚本的每一条守卫都在 docs/DEVELOPMENT_LEDGER.md 里记录了
    一次「故意违规 → 变红」的实证。

用法:
    python3 scripts/guards/policy_check.py            # 全部守卫, 违反即 exit 1
    python3 scripts/guards/policy_check.py --only g03  # 只跑某一条 (诊断用)
    python3 scripts/guards/policy_check.py --list      # 列出全部守卫

零第三方依赖: 只用标准库 (tomllib / pathlib / re)。
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# --- 规范事实表 (来源: docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md §8, AGENTS.md §2) ---

#: 严禁引入任何 GUI 依赖的引擎层 crate [ARCH-TOP-003, AGENTS.md 红线 3]
ENGINE_CRATES_NO_GUI = (
    "yeban-model",
    "yeban-dsp",
    "yeban-theory",
    "yeban-render",
    "yeban-engine",
    "yeban-sfz",
    "yeban-decode",
)

#: 强制 #![forbid(unsafe_code)] 的 crate [AGENTS.md 红线 8]
SAFE_BY_DEFAULT_CRATES = (
    "yeban-model",
    "yeban-theory",
    "yeban-dsp",
    "yeban-render",
)

#: GUI 依赖黑名单 (出现即违规)
GUI_DEP_PATTERNS = ("slint", "winit", "glutin", "egui", "qt_core", "gtk", "iced", "tao")

#: ⭐ R567: **非引擎层** crate 的显式清单 —— 与 `ENGINE_CRATES_NO_GUI` 合起来构成
#: "每个 crate 都必须被分类"的全集（`G15`）。⚠ 为什么需要：`G02`／`G03`／`G11` 都是
#: **白名单式**（遍历名字列表）⇒ **新增一个 crate 会静默逃逸**（既不受 GUI 禁令、
#: 也不必 `forbid(unsafe_code)`）—— 见 R566 的探针 crate 实测。
NON_ENGINE_CRATES = (
    "yeban-app",
    "yeban-diagnostics",
    "yeban-mcp",
    "yeban-midi",
    "yeban-midi-io",
    "yeban-plugin-host",
    "yeban-record",
    "yeban-services",
    "yeban-ui-mcp",
    "yeban-ui-test-port",
    "yeban-vst",
)

#: 官方默认 release 构建禁止默认开启的 feature [AGENTS.md 红线 6]
FORBIDDEN_DEFAULT_FEATURES = (
    "mcp-http",
    "ui-mcp",
    "asio",
    "experimental-vst3",
    "experimental-als-export",
    # `experimental-logic-export`: 实验性 Logic Pro `.logicx` bundle 导出。与上面同族
    # （实验性 DAW 互操作导出），必须永远保持"非默认"，故纳入本名单做机械防线。
    "experimental-logic-export",
    # 任务单 #143：只读 Logic bundle 探针（`--inspect-logic`）。与导出器同族 —— 解析 Apple 的
    # 私有分块二进制，只做结构对账、不产生 `.yeban`；必须永远保持"非默认"，故纳入本名单。
    "experimental-logic-import",
    # 账本第 800 轮：`yeban-app` 的进程内 MCP 挂载（新依赖边 + 监听端口）—— 与上面几项同类，
    # 必须永远保持"非默认"，故纳入本名单做机械防线（纵深防御）。
    "in-process-mcp",
)

#: 单文件大小上限 (字节); 超过必须登记 [AGENTS.md 红线 9]
MAX_FILE_BYTES = 10 * 1024 * 1024

#: 允许存在的大文件白名单 (相对仓库根); 新增需给出理由与许可登记
LARGE_FILE_ALLOWLIST: tuple[str, ...] = ()

SKIP_DIRS = {
    ".git",
    "target",
    "node_modules",
    ".worktrees",
    "dist",
    ".cargo-home",
    # R176 的加法半边: `__pycache__` 是 Python 产物, 而本仓**跟踪了 6 个**
    # `__pycache__/*.pyc` (`git ls-files | grep -c '__pycache__'` = 6)
    # ⇒ **任何一次本地 `import`** 都会重新生成它们 ⇒ 工作树变脏 ⇒
    # `git rebase` 拒绝(`Please commit or stash them`) ⇒ **推送被挡**
    # (我自己在做 R175 的等价性比对时就被挡了两次)。
    # 守卫本来也不扫 `.pyc`(后缀只认 .rs/.toml/.slint/.py/.sh), 所以这个加法
    # **不改变任何判定结果** —— 这一点由"改前/改后守卫输出逐字相同"证明。
    # 另一半("把产物移出版本控制")需要单独裁决, 见 R176 ①。
    "__pycache__",
}


def iter_repo_paths():
    """R175: 剪枝遍历 —— 语义等价于 `REPO.rglob("*")`, 但**不走进** SKIP_DIRS。

    动机 (mod-decode 批 17 实测, 已登记为候选): `rglob` 会**先走进** `target/`
    再在事后用 `rel_parts`/`SKIP_DIRS` 过滤 ⇒ 本工作区 `target/` 有 1.9 GB
    ⇒ 本地一次守卫跑要十几分钟。CI 上 checkout 干净所以看不出成本, 但那只是
    "这次没踩到", 不是"没有这个问题"。

    语义: 与 `rglob("*")` 一样**同时产出目录与文件**, 只是被剪掉的目录不再下探。
    调用点仍各自保留原有的 `is_file()` / 后缀 / `SKIP_DIRS` 判定, 所以结果是
    **同一个集合** —— 这一点由「改前/改后守卫判决逐字相同」来证明, 不靠断言。
    """
    import os

    for dirpath, dirnames, filenames in os.walk(REPO):
        # 原地裁剪 ⇒ os.walk 不会再下探这些目录。这才是"剪枝"; rglob 做不到。
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        base = Path(dirpath)
        for d in dirnames:
            yield base / d
        for f in filenames:
            yield base / f

Violation = tuple[str, str, str]  # (guard_id, location, message)


def rel(p: Path) -> str:
    try:
        return str(p.relative_to(REPO))
    except ValueError:
        return str(p)


def rel_parts(path: Path) -> tuple[str, ...]:
    """相对**仓库根**的路径分量（而不是绝对路径分量）。

    必须这样做的原因: 工作线在 `<main>/.worktrees/<line>/` 里运行, 绝对路径的每一段都含
    `.worktrees`。若用绝对分量做跳过判定, **工作线里所有源码/配置文件都会被跳过**,
    G04/G06/G07/G12 会变成永不报错的空判据 —— 那是最糟的一种"假绿"。
    """
    try:
        return path.resolve().relative_to(REPO.resolve()).parts
    except ValueError:
        return path.parts


def crates() -> list[Path]:
    out: list[Path] = []
    for parent in ("crates", "spikes"):
        base = REPO / parent
        if base.is_dir():
            out.extend(sorted(d for d in base.iterdir() if d.is_dir()))
    return out


def read_manifest(crate_dir: Path) -> dict:
    with (crate_dir / "Cargo.toml").open("rb") as fh:
        return tomllib.load(fh)


def iter_source_files(suffixes: tuple[str, ...] = (".rs", ".toml", ".slint")) -> list[Path]:
    out: list[Path] = []
    for p in iter_repo_paths():
        if not p.is_file():
            continue
        if any(part in SKIP_DIRS for part in rel_parts(p)):
            continue
        if p.suffix in suffixes:
            out.append(p)
    return out


def rust_source_before_tests(text: str) -> str:
    """截掉 `#[cfg(test)]` 之后的内容。

    约定: 测试模块位于文件末尾; 测试里允许使用 HashMap 之类的容器做夹具。
    生产代码路径才是红线所在。截断点用「最后一个 `#[cfg(test)]`」,
    这样即使文件里有多个测试模块也能正确覆盖。
    """
    idx = text.find("#[cfg(test)]")
    return text if idx < 0 else text[:idx]


def is_comment_line(line: str) -> bool:
    """判断一整行是否为注释。

    守卫只针对**代码**。规范正文里必然出现 "HashMap 禁令" / "0.0.0.0 禁令"
    这类字样, 把注释也算违规只会逼后来者去写绕过标记, 反而削弱守卫。
    这里只跳过整行注释 (含 `///` / `//!` / `#`); 行尾注释**不**跳过, 因为
    `let addr = "0.0.0.0:9316"; // 危险` 这种情况必须继续报红。
    """
    return line.lstrip().startswith(("//", "#"))


# ---------------------------------------------------------------------------
# 守卫实现
# ---------------------------------------------------------------------------


def g01_no_hashmap_in_model() -> list[Violation]:
    """[MODEL-AST-003 / AGENTS.md 红线 4] 持久化 AST 严禁 HashMap/HashSet。"""
    bad: list[Violation] = []
    target = REPO / "crates" / "yeban-model" / "src"
    pattern = re.compile(r"\b(HashMap|HashSet|hashbrown)\b")
    if not target.is_dir():
        return bad
    for path in sorted(target.rglob("*.rs")):
        body = rust_source_before_tests(path.read_text(encoding="utf-8"))
        for lineno, line in enumerate(body.splitlines(), start=1):
            if is_comment_line(line):
                continue
            if pattern.search(line):
                bad.append(("G01", f"{rel(path)}:{lineno}", f"HashMap/HashSet 违规: {line.strip()}"))
    return bad


def g02_no_gui_deps_in_engine_crates() -> list[Violation]:
    """[ARCH-TOP-003 / AGENTS.md 红线 3] 引擎层 crate 零 GUI 依赖。"""
    bad: list[Violation] = []
    for crate in crates():
        if crate.name not in ENGINE_CRATES_NO_GUI:
            continue
        manifest_path = crate / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        manifest = read_manifest(crate)
        for section in ("dependencies", "build-dependencies", "dev-dependencies"):
            for dep_name in manifest.get(section, {}) or {}:
                low = dep_name.lower().replace("-", "_")
                if any(pat in low for pat in GUI_DEP_PATTERNS):
                    bad.append(
                        (
                            "G02",
                            f"{rel(manifest_path)} [{section}]",
                            f"引擎层 crate `{crate.name}` 引入了 GUI 依赖 `{dep_name}`",
                        )
                    )
    return bad


def g03_safe_by_default_attribute() -> list[Violation]:
    """[AGENTS.md 红线 8] model/theory/dsp/render 必须 #![forbid(unsafe_code)]。"""
    bad: list[Violation] = []
    for crate in crates():
        if crate.name not in SAFE_BY_DEFAULT_CRATES:
            continue
        lib = crate / "src" / "lib.rs"
        if not lib.is_file():
            bad.append(("G03", rel(crate), f"缺少 src/lib.rs (crate {crate.name})"))
            continue
        text = lib.read_text(encoding="utf-8")
        if "#![forbid(unsafe_code)]" not in text:
            bad.append(("G03", rel(lib), "缺少 #![forbid(unsafe_code)]"))
    return bad


def g04_no_zero_zero_zero_zero_bind() -> list[Violation]:
    """[ARCH-SEC-002 / AGENTS.md 红线 5] 严禁绑定 0.0.0.0。"""
    bad: list[Violation] = []
    for path in iter_source_files((".rs", ".toml", ".slint")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), start=1):
            if is_comment_line(line):
                continue
            if "0.0.0.0" in line:
                bad.append(("G04", f"{rel(path)}:{lineno}", "出现 0.0.0.0 监听字面量"))
    return bad


def g05_forbidden_default_features() -> list[Violation]:
    """[AGENTS.md 红线 6] 危险 feature 不得默认开启。"""
    bad: list[Violation] = []
    for crate in crates():
        manifest_path = crate / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        manifest = read_manifest(crate)
        default = manifest.get("features", {}).get("default", []) or []
        for feature in default:
            if feature in FORBIDDEN_DEFAULT_FEATURES:
                bad.append(
                    (
                        "G05",
                        rel(manifest_path),
                        f"default feature 中禁止出现 `{feature}` (仅限显式启用)",
                    )
                )
    return bad


def g06_large_files_registered() -> list[Violation]:
    """[AGENTS.md 红线 9] 不得提交 >10MB 未登记二进制。"""
    bad: list[Violation] = []
    for path in iter_repo_paths():
        if not path.is_file() or any(part in SKIP_DIRS for part in rel_parts(path)):
            continue
        if path.stat().st_size <= MAX_FILE_BYTES:
            continue
        if rel(path) in LARGE_FILE_ALLOWLIST:
            continue
        bad.append(
            (
                "G06",
                rel(path),
                f"{path.stat().st_size} 字节 > 10MB 且不在 LARGE_FILE_ALLOWLIST 中",
            )
        )

    # 历史覆盖: 工作树干净**不代表历史里没有大对象** —— 红线 9 的对象是「已提交的二进制」,
    # 一次误提交即使在后续提交里删掉, 仍然留在所有 ref 可达的历史中。
    # 用 `git rev-list --objects --all` + `git cat-file --batch-check` 枚举**所有 ref 可达的 blob**。
    scanned = 0
    try:
        rev = subprocess.run(
            ["git", "rev-list", "--objects", "--all"],
            cwd=REPO, capture_output=True, text=True, timeout=300,
        )
        if rev.returncode == 0:
            cat = subprocess.run(
                ["git", "cat-file", "--batch-check=%(objecttype) %(objectsize) %(rest)"],
                cwd=REPO, input=rev.stdout, capture_output=True, text=True, timeout=300,
            )
            seen: set[str] = set()
            for line in cat.stdout.splitlines():
                parts = line.split(" ", 2)
                if len(parts) < 3 or parts[0] != "blob":
                    continue
                scanned += 1
                try:
                    size = int(parts[1])
                except ValueError:
                    continue
                if size <= MAX_FILE_BYTES:
                    continue
                hist_path = parts[2].strip()
                if hist_path in LARGE_FILE_ALLOWLIST or hist_path in seen:
                    continue
                seen.add(hist_path)
                bad.append(
                    (
                        "G06",
                        f"history:{hist_path}",
                        f"历史可达 blob {size} 字节 > 10MB 且不在 LARGE_FILE_ALLOWLIST 中",
                    )
                )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass

    # **见证**: 历史扫描必须真的读到非平凡的 blob 数。否则(例如 git 不可用、或扫描逻辑被改坏成永真)
    # 这条判据会变成"永不报错的空判据" —— 那正是本仓库最怕的假绿。
    if scanned < 100:
        bad.append(
            (
                "G06",
                "history-scan",
                f"历史扫描只读到 {scanned} 个 blob(<100) ⇒ 判据可能空转, 不能当作「红线 9 已满足」的证据",
            )
        )
    return bad


def g07_no_asio_sdk() -> list[Violation]:
    """[MUST-GATE-013 / ROAD-M-1-004] 仓库内不得出现 Steinberg ASIO SDK。"""
    bad: list[Violation] = []
    suspicious = re.compile(r"(asio[^a-z]*sdk|steinberg[^a-z]*asio|asio\.h$)", re.IGNORECASE)
    for path in iter_repo_paths():
        if any(part in SKIP_DIRS for part in rel_parts(path)):
            continue
        if suspicious.search(path.name):
            bad.append(("G07", rel(path), "疑似 ASIO SDK 文件"))
    return bad


def g08_workspace_inheritance_and_lints() -> list[Violation]:
    """[多线纪律] 成员 crate 必须继承 workspace 元数据并 opt-in 工作区 lint。"""
    bad: list[Violation] = []
    for crate in crates():
        manifest_path = crate / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        manifest = read_manifest(crate)
        package = manifest.get("package", {})
        for field in ("version", "edition", "license", "rust-version"):
            if field not in package:
                bad.append(("G08", rel(manifest_path), f"package.{field} 缺失"))
            elif isinstance(package[field], str) and not package[field].startswith("workspace"):
                bad.append(
                    (
                        "G08",
                        rel(manifest_path),
                        f"package.{field} 必须写作 `{field}.workspace = true`, 实际为字符串 `{package[field]}`",
                    )
                )
            elif isinstance(package[field], dict) and "workspace" not in package[field]:
                bad.append(("G08", rel(manifest_path), f"package.{field} 必须是 workspace 继承"))
        if manifest.get("lints", {}).get("workspace") is not True:
            bad.append(
                ("G08", rel(manifest_path), "缺少 [lints] workspace = true (无法继承工作区 lint 策略)")
            )
    return bad


def g09_glob_members_have_manifests() -> list[Violation]:
    """[Cargo.toml 纪律] crates/* 与 spikes/* 的每个目录都必须是合法 crate。"""
    bad: list[Violation] = []
    for parent in ("crates", "spikes"):
        base = REPO / parent
        if not base.is_dir():
            bad.append(("G09", parent, f"目录 {parent}/ 不存在 (workspace glob 成员要求它存在)"))
            continue
        for child in sorted(base.iterdir()):
            if child.is_dir() and not (child / "Cargo.toml").is_file():
                bad.append(
                    ("G09", rel(child), "目录中没有 Cargo.toml —— workspace glob 会因此解析失败")
                )
    return bad


def g10_no_wildcard_versions() -> list[Violation]:
    """[deny.toml wildcards=deny] 禁止 `*` 版本需求。"""
    bad: list[Violation] = []
    for crate in crates():
        manifest_path = crate / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, spec in (read_manifest(crate).get(section, {}) or {}).items():
                if isinstance(spec, str) and spec.strip().startswith("*"):
                    bad.append(("G10", rel(manifest_path), f"依赖 `{name}` 使用了通配版本 `{spec}`"))
    return bad


def g11_no_web_wasm_engine() -> list[Violation]:
    """[核心宪章 2 / RSK-27] 引擎层不得引入 wasm/web 运行时依赖。"""
    bad: list[Violation] = []
    banned = ("wasm-bindgen", "web-sys", "js-sys", "wasm-bindgen-futures")
    for crate in crates():
        if crate.name not in ENGINE_CRATES_NO_GUI and not crate.name.startswith("spike-"):
            continue
        manifest_path = crate / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        for section in ("dependencies", "build-dependencies"):
            for dep_name in (read_manifest(crate).get(section, {}) or {}):
                if dep_name in banned:
                    bad.append(
                        (
                            "G11",
                            rel(manifest_path),
                            f"`{crate.name}` 引入了 web/wasm 依赖 `{dep_name}`",
                        )
                    )
    return bad


def g12_no_tool_cache_in_tree() -> list[Violation]:
    """[仓库卫生 / 教训 L7] 工具缓存与日志 zip 绝不入仓库。

    背景: `gh run view --log` 会把 run-log zip 写进 `XDG_CACHE_HOME`。有一次缓存目录被指到
    仓库内, 于是 `git add -A` 把一个 424KB 的 CI 日志 zip 提交进了 main (现实中真发生过)。
    这条守卫保证它不再复发: 目录名与文件名两类都拦。
    """
    bad: list[Violation] = []
    cache_dirs = {".cache", ".wrangler", ".cargo-home", "node_modules", ".venv"}
    for path in iter_repo_paths():
        if not path.is_file():
            continue
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        rel_parts = path.relative_to(REPO).parts
        if any(part in cache_dirs for part in rel_parts):
            bad.append(("G12", rel(path), "工具缓存目录内的文件不应入库"))
            continue
        name = path.name
        if name.startswith("run-log-") and name.endswith(".zip"):
            bad.append(("G12", rel(path), "CI 日志 zip 不应入库 (缓存目录被指到仓库内的典型症状)"))
    return bad


def g13_workflows_are_valid() -> list[Violation]:
    """[CI 可用性] 所有 workflow 必须是合法 YAML, 且每个 job 都有 runs-on, 每个 workflow 都有触发条件。

    为什么值得机械化: 我自己在给手动档加 fuzz 档位时, 把一段含双引号的说明写进了双引号字符串,
    YAML 直接解析失败 —— 而这种错误在 GitHub 上表现为"整个 workflow 不出现/不触发",
    排查起来比编译错误贵得多。判据必须在本地就能变红。

    PyYAML 只在开发机/CI 上装 (python3 -c "import yaml"); 装不到时本守卫**跳过并出声**,
    而不是假装通过 —— 静默跳过等于假绿。
    """
    bad: list[Violation] = []
    wf_dir = REPO / ".github" / "workflows"
    if not wf_dir.is_dir():
        bad.append(("G13", ".github/workflows", "缺少 workflow 目录"))
        return bad
    try:
        import yaml  # noqa: PLC0415
    except ImportError:
        print("       (提示: 未安装 PyYAML, G13 未执行 —— 请 pip install pyyaml)")
        return bad

    files = sorted(wf_dir.glob("*.yml"))
    if not files:
        bad.append(("G13", ".github/workflows", "没有任何 .yml workflow"))
    for path in files:
        try:
            doc = yaml.safe_load(path.read_text(encoding="utf-8"))
        except yaml.YAMLError as exc:
            first = str(exc).splitlines()[0]
            bad.append(("G13", rel(path), f"YAML 解析失败: {first}"))
            continue
        if not isinstance(doc, dict):
            bad.append(("G13", rel(path), "顶层不是映射"))
            continue
        triggers = doc.get("on", doc.get(True))
        if not triggers:
            bad.append(("G13", rel(path), "缺少触发条件 (on:)"))
        jobs = doc.get("jobs") or {}
        if not jobs:
            bad.append(("G13", rel(path), "没有任何 job"))
        for name, job in jobs.items():
            # job **键**（id）必须是 GitHub 允许的字符集。实测事故: 我把 `goldens (生成分平台基准图集)` 当成 job 键,
            # python 的 yaml 解析器照样通过、run-gates.sh light 也全绿, 但 GitHub **拒绝派发整个 workflow**:
            #   "The identifier 'goldens (生成分平台基准图集)' is invalid. IDs may only contain alphanumeric
            #    characters, '_', and '-'"
            # 「本地绿、却根本发不出去」正是本仓库最怕的一类假绿, 所以这里按 GitHub 的规则机械判定。
            # 中文只能出现在 `name:` 里。
            if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,99}", str(name)):
                bad.append(
                    (
                        "G13",
                        f"{rel(path)}::{name}",
                        "job id 不合法(GitHub 只允许 [A-Za-z_][A-Za-z0-9_-]{0,99}); "
                        "中文/空格/括号请写在 name: 里 —— 否则整个 workflow 无法派发",
                    )
                )
            if not isinstance(job, dict):
                bad.append(("G13", f"{rel(path)}::{name}", "job 不是映射"))
                continue
            if "runs-on" not in job and "uses" not in job:
                bad.append(("G13", f"{rel(path)}::{name}", "job 缺少 runs-on"))

            # 步骤级观测性（第 126 轮加入，源自两次实测事故）:
            # **总是运行**的步骤若把 stdout 只重定向进 `$GITHUB_STEP_SUMMARY`，那它的产物在
            # `gh run view --log` 里**一个字都看不到** —— 失败时无从定位（`pending` 档就这样黑了很久）。
            # `if: failure()` 的"失败摘要"步骤不在此列: 它们抄的是失败步骤**自己日志里已有**的内容。
            for step in job.get("steps") or []:
                if not isinstance(step, dict):
                    continue
                run = step.get("run")
                if not isinstance(run, str) or "GITHUB_STEP_SUMMARY" not in run:
                    continue
                if "failure()" in str(step.get("if", "")):
                    continue
                if "tee" in run:
                    continue
                bad.append(
                    (
                        "G13",
                        f"{rel(path)}::{name}::{step.get('name', '?')}",
                        "总是运行的步骤把输出只送进 step summary(日志里看不到); "
                        "先 `} 2>&1 | tee <file>`, 再把同一份字节 cat 进 summary",
                    )
                )
    return bad


def _g14_python_syntax_ok() -> list[Violation]:
    """G14 的另一半: `scripts/**/*.py` 必须能被 `ast.parse` 解析。

    为什么加它（实测事故）: 我在会话里**第六次**把 ASCII 双引号写进双引号 Python 字符串,
    于是新建的工具脚本直接 `SyntaxError` —— 而当时**没有任何本地判据**会因此变红,
    只有"跑到那个脚本"才会发现。G14 已经对 shell 脚本做 `bash -n`, 这一半是它的对称物:
    工具脚本的语法错误必须在 `light` 阶段就被抓住。
    """
    import ast  # noqa: PLC0415

    bad: list[Violation] = []
    for path in sorted((REPO / "scripts").rglob("*.py")):
        try:
            ast.parse(path.read_text(encoding="utf-8"))
        except SyntaxError as exc:
            bad.append(("G14", f"{rel(path)}:{exc.lineno}", f"Python 语法错误: {exc.msg}"))
    return bad


def g14_shell_scripts_are_portable() -> list[Violation]:
    """[本地门禁可移植性] 仓库里的 shell 脚本必须在**最老的 bash**上也成立。

    为什么值得机械化（实测事故）: 我给 `run-gates.sh` 加"重依赖在 feature 后面则走轻量变体"时,
    顺手用了 `declare -A` 关联数组 —— 开发机 macOS 自带的是 **bash 3.2**, 它**没有关联数组**:
    `${ARR[yeban-engine]}` 会被当成**算术下标**去求值 `yeban`, 于是报
    `yeban: unbound variable`。**而 `bash -n` 语法检查照样通过** —— 也就是说,
    "语法没问题"完全掩盖了"在这个 shell 上跑不起来"。

    人类负责人的交互 shell 是 **zsh**(Ghostty), 但仓库脚本是被 `bash script.sh`、
    CI 的 `shell: bash`、以及各种 shebang 调用的 —— 调用者用什么 shell 不该决定脚本能不能跑。
    判据: ① 不得出现 `declare -A`/关联数组用法; ② 每个 `*.sh` 都能被 `bash -n` 解析。
    """
    bad: list[Violation] = []
    bash = shutil.which("bash")
    for path in sorted(REPO.rglob("*.sh")):
        parts = path.relative_to(REPO).parts
        if ".worktrees" in parts or "target" in parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), start=1):
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            if "declare -A" in stripped or "typeset -A" in stripped:
                bad.append((
                    "G14",
                    f"{rel(path)}:{lineno}",
                    "关联数组在 bash 3.2 上不存在（开发机就是 3.2）；用 case 表达同一件事",
                ))
        # ③ 运行时可移植性: `"${arr[@]}"` 在 bash 3.2 + `set -u` 下、数组为空时会炸。
        #    注意 `bash -n` **抓不到**它（它是运行时语义, 不是语法）—— 这正是这条检查存在的理由。
        if re.search(r"set -[a-z]*u", text):
            for lineno, line in enumerate(text.splitlines(), start=1):
                stripped = line.strip()
                if stripped.startswith("#"):
                    continue
                for match in re.finditer(r'"\$\{([A-Za-z_][A-Za-z0-9_]*)\[@\]\}"', line):
                    name = match.group(1)
                    # 允许两种安全写法: `${arr[@]+"${arr[@]}"}` 或 `"${arr[@]:-}"`
                    if f'${{{name}[@]+' in line or f'"${{{name}[@]:-}}"' in line:
                        continue
                    bad.append((
                        "G14",
                        f"{rel(path)}:{lineno}",
                        f'`"${{{name}[@]}}"` 在 bash 3.2 + set -u 下数组为空时会报 unbound; '
                        f'写成 `${{{name}[@]+"${{{name}[@]}}"}}`',
                    ))
        if bash is None:
            continue
        result = subprocess.run([bash, "-n", str(path)], capture_output=True, text=True)
        if result.returncode != 0:
            bad.append(("G14", rel(path), f"bash -n 失败: {result.stderr.strip()[:120]}"))
    return bad + _g14_python_syntax_ok()


def g15_every_crate_is_classified() -> list[Violation]:
    """[多线纪律 / 红线 3、8] `crates/*` 的每个 crate 都必须被显式分类。

    ⭐ R566 实测的缺口：`G02`（引擎层禁 GUI）与 `G03`（强制 `forbid(unsafe_code)`）
    都是**白名单式** —— 它们遍历 `ENGINE_CRATES_NO_GUI`／`SAFE_BY_DEFAULT_CRATES`
    这些**硬编码名单**。于是**新增**一个引擎层 crate 会**静默逃逸**：它可以加 `slint`
    依赖、也可以不写 `#![forbid(unsafe_code)]`，而两条守卫都不红。

    ⭐ 本条补法：要求每个 `crates/*` 落在**引擎层**或**非引擎层**之一。新增 crate
    ⇒ **立刻红** ⇒ 迫使人在**同一次提交里**分类它。
    """
    bad: list[Violation] = []
    base = REPO / "crates"
    if not base.is_dir():
        bad.append(("G15", "crates", "目录 crates/ 不存在"))
        return bad
    known = set(ENGINE_CRATES_NO_GUI) | set(NON_ENGINE_CRATES)
    if not known:
        bad.append(("G15", "crates", "分类集合为空 ⇒ 判据空转，不能当作「已分类」的证据"))
        return bad
    seen = 0
    for child in sorted(base.iterdir()):
        if not child.is_dir():
            continue
        seen += 1
        if child.name not in known:
            bad.append(
                (
                    "G15",
                    rel(child),
                    f"crate `{child.name}` 未被分类 ⇒ 请把它加入 ENGINE_CRATES_NO_GUI"
                    " 或 NON_ENGINE_CRATES（否则 G02/G03/G11 会静默漏掉它）",
                )
            )
    if seen == 0:
        bad.append(("G15", "crates", "crates/ 下没有任何 crate ⇒ 判据空转"))
    return bad


def g16_no_forbidden_feature_is_defined() -> list[Violation]:
    """[红线 6 / MUST-GATE-013] `asio` 与 `experimental-vst3` **永不定义**该 feature 名。

    ⭐ R591 的 ADR-0007 登记的限度：`asio` 的"不定义"原本**没有机械守卫**
    —— 因为它没有 `Cargo.toml` 条目可判，`FORBIDDEN_DEFAULT_FEATURES` 只管
    "一旦定义则禁进 default"，`G07` 只管"仓库内无 ASIO SDK 文件"。
    ⇒ 于是"有人新加一个 `asio` feature 但不开默认"在两者下**都不会红**。

    ⭐ 本条补上：任何 `Cargo.toml` 的 `[features]` 里**出现**这两个名字即红。
    ⚠ 口径依据：`AGENTS.md` 红线 6（⭐ 经 2026-10-15 人类裁决分写为两句）明确
    `asio` **不定义、不引入**；`experimental-vst3` 同属"尚未定义"类。
    """
    bad: list[Violation] = []
    never_defined = ("asio", "experimental-vst3")
    for crate in crates():
        manifest_path = crate / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        manifest = read_manifest(crate)
        feats = manifest.get("features", {}) or {}
        if not isinstance(feats, dict):
            continue
        for name in never_defined:
            if name in feats:
                bad.append(
                    (
                        "G16",
                        rel(manifest_path),
                        f"红线 6：`{name}` 是**永不定义**的禁止项（见 docs/adr/ADR-0007）"
                        f" ⇒ 不得在 `[features]` 里定义它",
                    )
                )
            for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                deps = manifest.get(section, {}) or {}
                if isinstance(deps, dict) and name in deps:
                    bad.append(
                        (
                            "G16",
                            rel(manifest_path),
                            f"红线 6：`{name}` 不得作为依赖出现（[{section}]）",
                        )
                    )
    return bad


def g17_hardware_markers_are_ignored() -> list[Violation]:
    """[人类裁决 R687] `[HARDWARE]` 标记必须落在 `#[ignore = "..."]` 里。

    ⚠ 为什么：真麦克风、实体 MIDI 键盘、真声卡这类判据在 CI 上跑不了。
    人类裁决（2026-10-15，见台账 R687）定为：**代码里注明 `[HARDWARE] 需真人手工验证`**、
    CI 跳过、不阻塞、由真人手工验证。

    ⭐ 本守卫把"跳过"机械化：凡 `crates/**` 的 `.rs` 里出现 `[HARDWARE]`，
    它必须位于一条 `#[ignore` 属性内 ⇒ 于是 `cargo test` 自然跳过它。
    ⛔ 只写注释不 ignore 的，本守卫报红 —— 因为那在 CI 上仍然会跑并变红。
    """
    out: list[Violation] = []
    for path in iter_source_files((".rs",)):
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines):
            if "[HARDWARE]" not in line:
                continue
            # ⭐ 粒度：本守卫的本意是**标记级**（真正的 #[ignore] 标记），而注释里
            # 引用该字面量是合法的（如"标记只写在 #[ignore] 的 reason 里"）。故跳过
            # 整行以 `//` 开头的注释 —— 它们不可能是属性本身。
            if line.strip().startswith("//"):
                continue
            # 允许的形态：本行、或**紧邻的上/下一行**是 #[ignore …] 属性的一部分
            window = lines[max(0, i - 1): min(len(lines), i + 2)]
            if any("#[ignore" in w for w in window):
                continue
            out.append(
                (
                    "G17",
                    f"{rel(path)}:{i + 1}",
                    "[HARDWARE] 不在 #[ignore] 内 ⇒ CI 上仍会跑（人类裁决 R687 要求 CI 跳过；"
                    '应写 `#[ignore = "[HARDWARE] 需真人手工验证 …"]`）',
                )
            )
    return out


#: 纪律族的号号上界（[`g18_ledger_rule_numbers_stay_in_one_family`]）。
#: 2026-10-16 的实测值：本族在 **278** 附近，每次会话长个位数 ⇒ 界放到 400。
LEDGER_RULE_CEILING = 400


def g18_ledger_rule_numbers_stay_in_one_family() -> list[Violation]:
    """[交接一致性] 台账里的「第 N 条」不得出现**外来号族**。

    为什么值得机械化（实测**四次**）: `scripts/tools/check_handover_consistency.py` 取
    `docs/ledger/phase-status.md` 里 `第 (\\d+) 条` 的**全局最大值**，再拿它去比交接 §4 标题
    声明的区间。于是**引用另一个号族**（例如别的台账的轮次号、或一句引文里的条号）
    会把这个最大值抬到几百 ⇒ 交接检查报「声明到第 278 条，而台账已到第 969 条」——
    ⚠ 而**没有一行规则出错**，排查成本全在"为什么最大的条号是 969"。
    修法是改引 `AGENTS.md §…`，⛔ 不写那个数字。

    ⚠ 本条**只查 `phase-status.md`** —— 那正是交接检查唯一读的文件（别的台账不参与对账）。

    判据: 每个 `第 N 条` 的 N ≤ [`LEDGER_RULE_CEILING`]。该上界是**结构性**的：
    一旦越界，要么混入了外来号族，要么有人真把纪律族写过了 400 —— 两种都值得人工看一眼。
    ⚠ 真的合法越过 ⇒ **在同一次提交里抬高本常数并注明**，⛔ 不是删掉本判据。
    """
    bad: list[Violation] = []
    path = REPO / "docs" / "ledger" / "phase-status.md"
    if not path.is_file():
        bad.append(("G18", "docs/ledger/phase-status.md", "台账文件不存在 ⇒ 本判据空转，不能算证据"))
        return bad
    text = path.read_text(encoding="utf-8")
    numbers = sorted({int(m) for m in re.findall(r"第\s*(\d+)\s*条", text)})
    if not numbers:
        bad.append(
            ("G18", "docs/ledger/phase-status.md", "读不到任何「第 N 条」⇒ 本判据空转，不能算证据")
        )
        return bad
    for n in numbers:
        if n > LEDGER_RULE_CEILING:
            bad.append(
                (
                    "G18",
                    f"docs/ledger/phase-status.md: 第 {n} 条",
                    f"纪律条号 {n} 超过本族上界 {LEDGER_RULE_CEILING} ⇒ 疑似混入外来号族；"
                    "应改引 `AGENTS.md §…`（⛔ 不要写那个数字 —— "
                    "`check_handover_consistency.py` 会据此误报交接陈旧）",
                )
            )
    return bad


#: 结构体/枚举**定义体**里"类型位置写了值"的形态 ⇒ `error[E0573]: expected type, found variant`。
#: 值一侧只列**绝不可能**是类型的起始符：`None`／`Some(`／`true`／`false`／数字／字符串。
VALUE_IN_TYPE = re.compile(
    r"""^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(\w+)\s*:\s*(None\b|Some\s*\(|true\b|false\b|-?\d|")"""
)
DEF_HEAD = re.compile(r"\b(?:pub(?:\s*\([^)]*\))?\s+)?(?:struct|enum)\s+(\w+)")


def _strip_code(line: str) -> str:
    """把 `//` 之后与 `"…"` 的内容抹掉（字符串保留一对引号），供括号配平用。

    ⚠ 只处理行注释与字符串；`/* … */` 块注释不处理（本仓的文档注释一律 `///`）。
    """
    out: list[str] = []
    instr = esc = False
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
            out.append('"' if ch == '"' and not instr else " ")
        elif ch == '"':
            instr = True
            out.append('"')
        elif line.startswith("//", i):
            break
        else:
            out.append(ch)
        i += 1
    return "".join(out)


def iter_definition_bodies(text: str):
    """产出 `(名字, [(行号, 原始行), …])`：**顶层** `struct`/`enum` 的定义体。

    口径：定义头必须出现在括号深度 0（即不在 `fn`／`impl`／`macro_rules!` 体内），
    且 `{` 必须与定义头**同行**（rustfmt 的默认形态）；`struct X;` 这类无体单元结构体不产出。
    ⚠ 局部（函数体内的）定义不扫 —— 本仓没有。
    """
    depth = 0
    name: str | None = None
    body: list[tuple[int, str]] = []
    for ln, raw in enumerate(text.split("\n"), 1):
        code = _strip_code(raw)
        if name is None and depth == 0:
            m = DEF_HEAD.search(code)
            if m and "{" in code[code.index(m.group(0)) :]:
                name = m.group(1)
        if name is not None:
            body.append((ln, raw))
        depth += code.count("{") - code.count("}")
        if name is not None and depth <= 0:
            yield name, body
            name = None
            body = []
            depth = 0


def g19_no_value_in_struct_type_position() -> list[Violation]:
    """[本机可判定编译错] 定义体里的字段类型不得写成值（`field: None` ⇒ E0573）。

    为什么值得机械化：2026-10-28 的 `G8n`（`beatGrouping`）用脚本给 35 处 `PlanOptions`
    字面量补 `beat_grouping: None,`，其中两行落进了**别的结构体**的定义体与 `Self { … }`
    字面量 ⇒ 推送后三条代码腿同红 `E0573`，一轮 CI ≈ 45 分钟。
    ⚠ `audit_struct_literals.py` 抓不到它：那个工具只对账 `PlanOptions` 字面量的**键集**，
    误插的目标根本不是 `PlanOptions`。本守卫按**定义体**扫，因而是另一条独立防线。

    判据：`struct`/`enum` 定义体（到配对 `}` 为止）内，任何 `名字: <值>` 行都算违规。
    反例（必须报）：`struct A { x: u8, beat_grouping: None, }`；
    正例（⛔ 报）：`struct A { x: Option<u8>, }`、`let a = A { x: None };`（那是字面量）。
    """
    bad: list[Violation] = []
    for p in iter_source_files((".rs",)):
        try:
            text = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for name, body in iter_definition_bodies(text):
            for ln, raw in body:
                if is_comment_line(raw):
                    continue
                m = VALUE_IN_TYPE.match(_strip_code(raw))
                if m:
                    bad.append(
                        (
                            "G19",
                            f"{rel(p)}:{ln}",
                            f"`{name}` 的定义体里 `{m.group(1)}` 的类型位置写成了值 "
                            f"`{m.group(2).strip()}` ⇒ `E0573 expected type, found variant`"
                            "（补位脚本误插的典型形态；值只能出现在字面量里）",
                        )
                    )
    return bad


ADR_DIR = REPO / "docs" / "adr"
ADR_INDEX = ADR_DIR / "README.md"
ADR_LINK = re.compile(r"\]\(\./(ADR-[^)\s]+\.md)\)")


def g20_adr_index_matches_adr_files() -> list[Violation]:
    """[文档不变量] `docs/adr/README.md` 的索引与 `docs/adr/ADR-*.md` 双向一致。

    为什么值得机械化：ADR 的索引行是本仓库里**唯一**的发现入口，而交接反复规定
    "索引行要在 ADR 落到主干**之后**补（⛔ 提前写＝死链）"。这条纪律此前只靠人记 ——
    `G8n`（`ADR-0046`）就得在 land 之后单独补一次索引行，且没有任何东西会拦下一次漏补。

    判据两条，都能变红：
    ① 每个 `ADR-*.md` 文件都必须在索引里有一行 `](./文件名)`（⛔ 漏行 ⇒ 文档不可发现）；
    ② 索引里每个 `](./ADR-*.md)` 都必须指向**存在**的文件（⛔ 死链 ⇒ "提前写索引行"的形态）。
    口径：只比对相对链接 `](./ADR-….md)`；README 正文里不带链接的文件名提及不参与。
    """
    if not ADR_INDEX.is_file():
        return [("G20", rel(ADR_INDEX), "索引文件缺失 ⇒ `docs/adr/` 下的 ADR 无法被发现")]
    try:
        text = ADR_INDEX.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError) as exc:
        return [("G20", rel(ADR_INDEX), f"索引不可读: {exc}")]
    files = {p.name for p in ADR_DIR.glob("ADR-*.md")}
    linked = set(ADR_LINK.findall(text))
    bad: list[Violation] = []
    for name in sorted(files - linked):
        bad.append(
            (
                "G20",
                rel(ADR_DIR / name),
                f"`{name}` 在 `docs/adr/README.md` 里没有索引行 ⇒ 文档不可发现"
                f"（补一行 `](./{name})`；⚠ 只能在 ADR 已落主干之后补）",
            )
        )
    for name in sorted(linked - files):
        bad.append(
            (
                "G20",
                rel(ADR_INDEX),
                f"索引行指向不存在的 `{name}` ⇒ 死链（索引行⛔ 提前写：等 ADR 落地同批补）",
            )
        )
    return bad


GUARDS = {
    "G01": ("[MODEL-AST-003] 持久化 AST 零 HashMap/HashSet", g01_no_hashmap_in_model),
    "G02": ("[ARCH-TOP-003] 引擎层 crate 零 GUI 依赖", g02_no_gui_deps_in_engine_crates),
    "G03": ("[红线 8] model/theory/dsp/render 强制 forbid(unsafe_code)", g03_safe_by_default_attribute),
    "G04": ("[ARCH-SEC-002] 严禁绑定 0.0.0.0", g04_no_zero_zero_zero_zero_bind),
    "G05": ("[红线 6] 危险 feature 不得默认开启", g05_forbidden_default_features),
    "G06": ("[红线 9] 无 >10MB 未登记文件", g06_large_files_registered),
    "G07": ("[MUST-GATE-013] 仓库内无 ASIO SDK", g07_no_asio_sdk),
    "G08": ("[多线纪律] workspace 元数据继承 + lint opt-in", g08_workspace_inheritance_and_lints),
    "G09": ("[多线纪律] glob 成员目录必须有 Cargo.toml", g09_glob_members_have_manifests),
    "G10": ("[deny.toml] 禁止通配版本", g10_no_wildcard_versions),
    "G11": ("[宪章 2] 引擎层零 web/wasm 依赖", g11_no_web_wasm_engine),
    "G12": ("[仓库卫生] 工具缓存与 CI 日志 zip 不入库", g12_no_tool_cache_in_tree),
    "G13": ("[CI 可用性] workflow YAML 合法且 job 完整", g13_workflows_are_valid),
    "G14": ("[本地门禁可移植性] shell 脚本 bash 3.2 兼容(禁关联数组)且 bash -n 可解析", g14_shell_scripts_are_portable),
    "G15": ("[多线纪律/红线 3、8] crates/* 每个 crate 都必须被显式分类", g15_every_crate_is_classified),
    "G16": ("[红线 6] asio／experimental-vst3 永不定义（ADR-0007）", g16_no_forbidden_feature_is_defined),
    "G17": ("[人类裁决 R687] [HARDWARE] 标记必须落在 #[ignore] 里", g17_hardware_markers_are_ignored),
    "G18": (
        "[交接一致性] 台账的「第 N 条」不得混入外来号族（R777 实测四次）",
        g18_ledger_rule_numbers_stay_in_one_family,
    ),
    "G19": (
        "[本机可判定编译错] struct/enum 定义体里不得把值写在类型位置（E0573，G8n 实测一次）",
        g19_no_value_in_struct_type_position,
    ),
    "G20": (
        "[文档不变量] docs/adr/README.md 索引与 ADR 文件双向一致（⛔ 漏行／⛔ 死链）",
        g20_adr_index_matches_adr_files,
    ),
}




def self_test() -> int:
    """守卫自检 —— 本仓的守卫**本来没有自检**（R546 的发现）。

    ⚠ 为什么需要：CI 会跑守卫、**但没有任何注入／变异测试** ⇒ 一条守卫若因重构
    或改错而**静默失效**（被从注册表里删掉、或函数抛异常／返回 None）、CI 会一直绿。
    ⭐ 本自检能机械抓住的三件事：
      ① **注册表完整性**：`G01`…`G20` 一个不缺、且没有空标题／非可调用项；
      ② **可调用性**：每个 check 函数存在且可调用；
      ③ **真的跑过并返回列表**：逐个调用，断言返回的是 `list`（不是 None、不抛异常）。
    ⚠ 它**不**证明每条判据对**坏输入**会报红（那需要对每个判据构造变异输入）；
    但那已强于"什么都没验"—— 至少"守卫被删掉／写坏"会立刻报出来。
    """
    fails: list[str] = []

    expected = [f"G{n:02d}" for n in range(1, 21)]
    missing = [g for g in expected if g not in GUARDS]
    if missing:
        fails.append(f"注册表缺少守卫 {missing}（共应有 {len(expected)} 条）")
    extra = [g for g in GUARDS if g not in expected]
    if extra:
        # ⭐ 多出来的不算错（可能新增），只提示
        print(f"[note] 注册表另有 {extra}")

    if not GUARDS:
        fails.append("注册表为空 ⇒ 本判据空转，不能当作「红线守卫在位」的证据")

    for gid, entry in GUARDS.items():
        if not (isinstance(entry, tuple) and len(entry) == 2):
            fails.append(f"{gid} 的注册项不是 (说明, 函数) 二元组")
            continue
        desc, fn = entry
        if not isinstance(desc, str) or not desc.strip():
            fails.append(f"{gid} 缺少文字说明")
        if not callable(fn):
            fails.append(f"{gid} 的 check 不是可调用对象")
            continue
        try:
            found = fn()
        except Exception as exc:  # noqa: BLE001 —— 自检要如实报出任何异常
            fails.append(f"{gid} 调用时抛异常: {exc!r}")
            continue
        if not isinstance(found, list):
            fails.append(f"{gid} 返回了 {type(found).__name__} 而不是 list ⇒ 判据可能失效")

    if fails:
        for f in fails:
            print(f"[FAIL] {f}", file=sys.stderr)
        print(f"[FAIL] 守卫自检 {len(fails)} 项不符 ⇒ 守卫的绿不算证据", file=sys.stderr)
        return 1
    print(
        f"[ok] 守卫自检通过：注册表 {len(GUARDS)} 条齐备（{expected[0]}–{expected[-1]}）、"
        "逐条可调用且返回列表"
    )
    print("[note] ⚠ 本自检不构造变异输入 ⇒ 它证明「守卫都在且能跑」，⛔ 不证明「每条都能报红」")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="夜半机械红线守卫")
    parser.add_argument("--only", action="append", default=None, help="只运行指定守卫 ID")
    parser.add_argument("--list", action="store_true", help="列出全部守卫")
    parser.add_argument("--self-test", action="store_true", help="自检：守卫是否齐备且可调用")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    if args.list:
        for gid, (desc, _) in GUARDS.items():
            print(f"{gid}  {desc}")
        return 0

    selected = args.only if args.only else list(GUARDS)
    unknown = [g for g in selected if g not in GUARDS]
    if unknown:
        print(f"未知守卫 ID: {', '.join(unknown)}", file=sys.stderr)
        return 2

    all_violations: list[Violation] = []
    for gid in selected:
        desc, fn = GUARDS[gid]
        found = fn()
        status = "FAIL" if found else "ok  "
        print(f"[{status}] {gid} {desc}" + (f"  ({len(found)} 处)" if found else ""))
        for _gid, location, message in found:
            print(f"         - {location}: {message}")
        all_violations.extend(found)

    if all_violations:
        print(
            f"\n守卫未通过: {len(all_violations)} 处违规。"
            " 依据 AGENTS.md §2 红线，这些是一票否决项，不允许 `#[allow]` 绕过。",
            file=sys.stderr,
        )
        return 1
    print(f"\n守卫全部通过 ({len(selected)} 条)。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
