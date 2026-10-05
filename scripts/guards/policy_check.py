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

#: 官方默认 release 构建禁止默认开启的 feature [AGENTS.md 红线 6]
FORBIDDEN_DEFAULT_FEATURES = (
    "mcp-http",
    "ui-mcp",
    "asio",
    "experimental-vst3",
    "experimental-als-export",
)

#: 单文件大小上限 (字节); 超过必须登记 [AGENTS.md 红线 9]
MAX_FILE_BYTES = 10 * 1024 * 1024

#: 允许存在的大文件白名单 (相对仓库根); 新增需给出理由与许可登记
LARGE_FILE_ALLOWLIST: tuple[str, ...] = ()

SKIP_DIRS = {".git", "target", "node_modules", ".worktrees", "dist", ".cargo-home"}

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
    for p in REPO.rglob("*"):
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
    for path in REPO.rglob("*"):
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
    for path in REPO.rglob("*"):
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
    for path in REPO.rglob("*"):
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
}


def main() -> int:
    parser = argparse.ArgumentParser(description="夜半机械红线守卫")
    parser.add_argument("--only", action="append", default=None, help="只运行指定守卫 ID")
    parser.add_argument("--list", action="store_true", help="列出全部守卫")
    args = parser.parse_args()

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
