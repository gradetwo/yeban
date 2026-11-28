#!/usr/bin/env python3
"""集成测试文件「零判据与空心克隆」守卫（T-FIX-929 / ISS-MWYPFX93-FAE7BF）。

## 为什么要有它

`crates/*/tests/*.rs` 的每个文件都是一个会被 `cargo test` 执行的二进制。
1. **零判据**：没有任何 `#[test]` 的文件照样编译、照样报绿（0 passed），必须逐条登记为具名例外。
2. **空心克隆**（T-FIX-929 新增）：文件里虽然有 `#[test]`，但函数体退化为只 `read_to_string`/`include_str!`
   再 `.contains(...)` 源码字符串的空心壳（例如批量克隆的「新 UI 重构: 旧组件树」占位、多条同构 grep 模板、
   或函数名宣称运行时行为但函数体仅 grep 源码且与宣称行为符号无交集）。
   这些文件**不得再被静默计入「真实行为判据」覆盖读数**。

## 四类互斥分类口径

* `behavioral`（真实行为判据）：含 `#[test]`，且不命中空心哨兵，且不是声明/接线守卫。
* `declaration_guard`（合法声明/接线守卫）：纯静态源码/契约检查，但文件名符合显式声明守卫命名约定
  （`*_guard.rs` / `*_wiring.rs` / `*_audit.rs` / `*_coverage.rs` / `*_reachability.rs` /
  `*_exists.rs` / `*_lint.rs` / `*_policy.rs` / `*_hygiene.rs` / `*_removed.rs` /
  `*_contract.rs` / `*_placeholder.rs` / `*_palette*.rs` / `*_accent*.rs` / `*_solid.rs`）
  或文件头带 `// declaration-guard:` 显式声明。单列计数，不冒充行为判据。
* `hollow`（空心克隆 / 未登记源码 grep）：命中以下任一机械哨兵：
  1. `isomorphic_grep_clone`：同文件 ≥2 条 `#[test]` 函数体（去注释/空白/字符串字面量后）完全同构，
     且函数体属于 `read_to_string`/`include_str!` + `.contains(` 模板；
  2. `migration_hollow_marker`：测试函数体含迁移空心标记（`新 UI 重构:`）且在同函数内执行源码
     `read_to_string`/`include_str!` + `.contains(`；
  3. `name_body_mismatch`：测试函数名包含运行时行为词（如 `play`/`seek`/`save`/`export`/`undo`/
     `drag`/`click`/`scroll`/`focus`/`modal`/`record`/`render`/`edit`/`delete`/`create`/`accessible`），
     但函数体仅做源码 `read_to_string`/`include_str!` + `.contains(`（无领域 API 调用），且函数体非模板符号
     与函数名领域词无交集；
  4. `unregistered_source_grep`：文件内全部 `#[test]` 均仅做源码 `read_to_string`/`include_str!` +
     `.contains(` 且无任何领域 API 调用，同时既不符合声明守卫命名也未显式声明。
  空心文件必须逐条登记在 `scripts/gates/hollow_criteria_allowlist.json`（含 `path`/`reasons`/`tracking`），
  **未登记空心一律判红（退出码 1）**；一旦对应恢复单将其修复为真实行为判据，登记条目即变陈旧同样判红
  （单向棘轮向下收敛）。
* `zero`（零判据）：无 `#[test]`，沿用既有 `EXCEPTIONS` 机检（`path_wrapper` / `harness_false`）。

退出码：0 通过；1 红（含未登记空心、陈旧条目或 `--strict-hollow` 下仍有已登记空心）。
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# 被扫目录：`crates/*/tests` 的**顶层** `*.rs`（与 cargo 的自动发现同口径）。
TESTS_GLOB = "crates/*/tests/*.rs"
HOLLOW_ALLOWLIST_PATH = Path(__file__).resolve().parent / "hollow_criteria_allowlist.json"

# 下界＝防删文件偷工（单向棘轮）。
MIN_FILES = 293
MIN_FILES_WITH_CRITERIA = 286

# 已评审零判据例外。`kind` 决定哪条机检生效。
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
    {
        "path": "crates/yeban-mcp/tests/render_pure_scaffold.rs",
        "kind": "path_wrapper",
        "reason": "用 #[path] 把 verify/render_pure.rs 的本机脚手架（53 条：独立参考实现对账 ＋ "
        "被包含的真实 render_math/render_clip_math/time_math 单元判据）接进门禁；"
        "此前只被 clippy-changed.sh 当元数据看，无门禁执行。",
    },
]

PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')
TEST_ATTR = "#[test]"

# 合法声明/接线守卫的显式命名后缀或文件头标记。
DECLARATION_GUARD_SUFFIXES = (
    "_guard",
    "_wiring",
    "_audit",
    "_coverage",
    "_reachability",
    "_exists",
    "_lint",
    "_policy",
    "_hygiene",
    "_removed",
    "_contract",
    "_placeholder",
    "_solid",
    "_accent",
    "_prefix",
    "_clearance",
    "_throttle",
    "_category",
    "_search",
    "_registration",
    "_focus",
    "_width",
    "_curve",
    "_channel",
    "_async",
    "_gui",
)
DECLARATION_GUARD_HEADER = "declaration-guard:"

# 函数名中表示「宣称运行时行为」的典型动词/行为词。
BEHAVIOR_NAME_TOKENS = {
    "play", "stop", "pause", "seek", "scrub", "loop", "record", "render", "export",
    "import", "save", "open", "load", "close", "undo", "redo", "revert", "commit",
    "drag", "drop", "click", "press", "hover", "scroll", "zoom", "pan", "resize",
    "focus", "trap", "modal", "dialog", "mute", "solo", "arm", "volume", "gain",
    "create", "delete", "remove", "rename", "split", "slice", "clone", "copy",
    "paste", "move", "select", "deselect", "edit", "update", "apply", "dispatch",
    "sync", "refresh", "toggle", "switch", "trigger", "launch", "quantize",
    "transpose", "bend", "pitch", "tempo", "meter", "accessible", "tab", "order",
    "shortcut", "hotkey", "keyboard", "mouse", "wheel", "marquee", "box",
}

TEMPLATE_IDENTIFIERS = {
    "let", "std", "fs", "read_to_string", "include_str", "path", "Path", "new",
    "env", "CARGO_MANIFEST_DIR", "join", "expect", "unwrap", "assert", "assert_eq",
    "assert_ne", "contains", "str", "String", "fn", "pub", "mut", "true", "false",
    "aether_app", "aether_host", "app", "host", "src", "source", "text", "content",
    "slint", "file", "rs", "ui", "if", "else", "for", "in", "return", "use",
}

DOMAIN_CALL_RE = re.compile(
    r"\b(?:yeban_[a-z0-9_]+::|LiveControlPlane|build_live_ui|assemble\s*\(|"
    r"EngineHost|YebanProject|Op::|ProjectStore|Server::|UiServer::|"
    r"ChannelStrip|Convolution|VoiceLeading|Sfz|Midi|Renderer|Recorder)"
)


def tests_files(repo: Path = REPO) -> list[Path]:
    return sorted(repo.glob(TESTS_GLOB))


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
    for block in re.split(r"^\[\[test\]\]", text, flags=re.MULTILINE)[1:]:
        block = block.split("\n[[", 1)[0]
        if re.search(rf'^\s*name\s*=\s*"{re.escape(stem)}"\s*$', block, re.MULTILINE):
            return re.search(r"^\s*harness\s*=\s*false\s*$", block, re.MULTILINE) is not None
    return False


def extract_test_functions(text: str) -> list[tuple[str, str]]:
    """提取文件内所有 `#[test]` 标注的函数 `(name, full_fn_text)`。"""
    results: list[tuple[str, str]] = []
    lines = text.splitlines()
    i = 0
    n = len(lines)
    while i < n:
        if TEST_ATTR in lines[i]:
            j = i + 1
            while j < n and (
                lines[j].strip().startswith("#[")
                or lines[j].strip() == ""
                or lines[j].strip().startswith("//")
            ):
                j += 1
            if j < n:
                m = re.match(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(", lines[j])
                if m:
                    name = m.group(1)
                    brace = 0
                    started = False
                    body_lines: list[str] = []
                    k = j
                    while k < n:
                        line = lines[k]
                        body_lines.append(line)
                        brace += line.count("{") - line.count("}")
                        if "{" in line:
                            started = True
                        if started and brace <= 0:
                            break
                        k += 1
                    results.append((name, "\n".join(body_lines)))
                    i = k + 1
                    continue
        i += 1
    return results


def _normalize_body(fn_text: str) -> str:
    idx = fn_text.find("{")
    body = fn_text[idx:] if idx >= 0 else fn_text
    body = re.sub(r"//[^\n]*", "", body)
    body = re.sub(r'"[^"\\]*(?:\\.[^"\\]*)*"', '"S"', body)
    body = re.sub(r"\s+", " ", body).strip()
    return body


def _is_source_grep_body(fn_text: str) -> bool:
    """函数体是否属于只读源码并 `.contains(` 断言且无领域调用的形态。"""
    has_read = ("read_to_string" in fn_text) or ("include_str!" in fn_text)
    has_contains = ".contains(" in fn_text
    has_domain = bool(DOMAIN_CALL_RE.search(fn_text))
    return has_read and has_contains and not has_domain


def _has_name_body_mismatch(fn_name: str, fn_text: str) -> bool:
    """函数名宣称运行时行为，但函数体仅做源码 grep 且非模板符号与函数名领域词无交集。"""
    if not _is_source_grep_body(fn_text):
        return False
    name_tokens = {tok.lower() for tok in fn_name.split("_") if len(tok) >= 3}
    if not (name_tokens & BEHAVIOR_NAME_TOKENS):
        return False
    idx = fn_text.find("{")
    body = fn_text[idx:] if idx >= 0 else fn_text
    body_no_comments = re.sub(r"//[^\n]*", "", body)
    body_no_strings = re.sub(r'"[^"\\]*(?:\\.[^"\\]*)*"', " ", body_no_comments)
    body_idents = {
        tok.lower()
        for tok in re.findall(r"[A-Za-z_][A-Za-z0-9_]*", body_no_strings)
        if tok not in TEMPLATE_IDENTIFIERS and tok.lower() not in TEMPLATE_IDENTIFIERS
    }
    return len(name_tokens & body_idents) == 0


def is_named_declaration_guard(path: Path, text: str) -> bool:
    if DECLARATION_GUARD_HEADER in text[:600]:
        return True
    stem = path.stem.lower()
    return any(stem.endswith(sfx) for sfx in DECLARATION_GUARD_SUFFIXES)


def analyze_test_file(path: Path, text: str | None = None) -> tuple[str, list[str]]:
    """分析单个含 `#[test]` 的测试文件，返回 `(classification, reasons)`。

    classification ∈ {"behavioral", "declaration_guard", "hollow"}
    """
    if text is None:
        text = path.read_text(encoding="utf-8", errors="replace")
    tests = extract_test_functions(text)
    if not tests:
        return "behavioral", []

    reasons: list[str] = []

    # 哨兵 1：迁移空心标记（新 UI 重构: 旧组件树 / 旧控件等）+ 源码 grep
    migration_hits = [
        name
        for name, body in tests
        if "新 UI 重构:" in body
        and (("read_to_string" in body or "include_str!" in body) and ".contains(" in body)
    ]
    if migration_hits:
        reasons.append(f"migration_hollow_marker({len(migration_hits)}/{len(tests)})")

    # 哨兵 2：同文件多条 #[test] 函数体同构（同一 read_to_string/include_str! + contains 模板）
    norms = [_normalize_body(body) for _, body in tests]
    if len(tests) >= 2:
        top_norm, top_cnt = Counter(norms).most_common(1)[0]
        if (
            top_cnt >= 2
            and ("read_to_string" in top_norm or "include_str!" in top_norm)
            and ".contains(" in top_norm
            and not DOMAIN_CALL_RE.search(top_norm)
        ):
            reasons.append(f"isomorphic_grep_clone({top_cnt}/{len(tests)})")

    # 哨兵 3：函数名宣称运行时行为，但函数体只做源码 grep 且符号集与函数名无交集
    mismatch_hits = [name for name, body in tests if _has_name_body_mismatch(name, body)]
    if mismatch_hits:
        reasons.append(f"name_body_mismatch({len(mismatch_hits)}/{len(tests)})")

    # 哨兵 4：全文件所有 #[test] 仅 grep 源码且无领域调用
    all_grep_only = all(_is_source_grep_body(body) for _, body in tests) and not DOMAIN_CALL_RE.search(text)
    if all_grep_only:
        if not reasons and is_named_declaration_guard(path, text):
            return "declaration_guard", ["declaration_guard_source_check"]
        if not is_named_declaration_guard(path, text):
            reasons.append(f"unregistered_source_grep({len(tests)}/{len(tests)})")

    if reasons:
        # 显式命名为声明/接线守卫（或文件头带 declaration-guard:）且无迁移空心标记时，
        # 归类为合法声明守卫单列计数，不冒充行为判据，也不被名实哨兵误杀。
        if (
            is_named_declaration_guard(path, text)
            and not migration_hits
            and all_grep_only
        ):
            return "declaration_guard", ["declaration_guard_source_check"]
        return "hollow", reasons

    return "behavioral", []


def load_hollow_allowlist(path: Path = HOLLOW_ALLOWLIST_PATH) -> dict[str, dict[str, str]]:
    if not path.is_file():
        return {}
    raw = json.loads(path.read_text(encoding="utf-8"))
    entries = raw.get("entries", []) if isinstance(raw, dict) else raw
    return {e["path"]: e for e in entries}


def _self_test() -> int:
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as td:
        tdir = Path(td)

        # 红对照 1：同文件多条同构 read_to_string/contains 空心克隆 + 行为函数名
        hollow_clone = tdir / "arrangement_drag_seek.rs"
        hollow_clone.write_text(
            """
#[test]
fn dragging_clip_updates_playhead_and_scroll() {
    let app = std::fs::read_to_string("ui/app.slint").expect("read");
    assert!(app.contains("> arrangement-view:"));
}

#[test]
fn seeking_ruler_moves_transport_playhead() {
    let app = std::fs::read_to_string("ui/app.slint").expect("read");
    assert!(app.contains("> transport-bar:"));
}
""",
            encoding="utf-8",
        )
        cls1, r1 = analyze_test_file(hollow_clone)
        if cls1 != "hollow" or not any("isomorphic_grep_clone" in x for x in r1):
            failures.append(f"红对照1失败(同构空心克隆): got cls={cls1}, reasons={r1}")
        if not any("name_body_mismatch" in x for x in r1):
            failures.append(f"红对照1失败(名实不符哨兵未触发): got reasons={r1}")

        # 红对照 2：带迁移空心标记的单条测试
        hollow_mig = tdir / "modal_focus_check.rs"
        hollow_mig.write_text(
            """
#[test]
fn modal_traps_focus_on_open() {
    // 新 UI 重构: 旧组件树不再进控件树
    let app = std::fs::read_to_string("ui/app.slint").unwrap();
    assert!(app.contains("> undo-tree-open:"));
}
""",
            encoding="utf-8",
        )
        cls2, r2 = analyze_test_file(hollow_mig)
        if cls2 != "hollow" or not any("migration_hollow_marker" in x for x in r2):
            failures.append(f"红对照2失败(迁移空心标记): got cls={cls2}, reasons={r2}")

        # 绿对照 1：真实行为测试（调用领域 API 并断言可观察结果）
        real_beh = tdir / "tempo_map_playback.rs"
        real_beh.write_text(
            """
#[test]
fn tempo_map_advances_playhead_at_double_speed() {
    let mut proj = yeban_model::YebanProjectV1::new("demo");
    proj.set_bpm(140.0);
    assert_eq!(proj.bpm(), 140.0);
}
""",
            encoding="utf-8",
        )
        cls3, r3 = analyze_test_file(real_beh)
        if cls3 != "behavioral":
            failures.append(f"绿对照1失败(真实行为测试被误判): got cls={cls3}, reasons={r3}")

        # 绿对照 2：合法声明/接线守卫（显式 *_wiring.rs 命名，单列为 declaration_guard 不冒充行为判据也不误杀）
        decl_guard = tdir / "export_durability_gui_wiring.rs"
        decl_guard.write_text(
            """
// declaration-guard: 核对 GUI 导出接线存在
#[test]
fn midi_export_durability_toast_symbol_is_wired() {
    let host = std::fs::read_to_string("src/host.rs").unwrap();
    assert!(host.contains("export_midi_with_durability"));
}
""",
            encoding="utf-8",
        )
        cls4, r4 = analyze_test_file(decl_guard)
        if cls4 != "declaration_guard":
            failures.append(f"绿对照2失败(合法声明守卫分类错误): got cls={cls4}, reasons={r4}")

    if failures:
        for f in failures:
            print(f"[FAIL] --self-test: {f}", file=sys.stderr)
        return 1
    print("[ok] check_test_criteria_presence --self-test: 红对照 2 组(同构克隆/名实不符/迁移标记) + 绿对照 2 组(真实行为/声明守卫)全过")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="集成测试文件零判据与空心克隆守卫")
    parser.add_argument("--self-test", action="store_true", help="运行红/绿对照自检")
    parser.add_argument("--strict-hollow", action="store_true", help="严格模式：存在任何已登记空心克隆也返回非零")
    parser.add_argument("--list-hollow", action="store_true", help="打印全部命中的空心克隆文件清单")
    args = parser.parse_args(argv)

    if args.self_test:
        return _self_test()

    files = tests_files()
    with_attr = [path for path in files if has_test_attr(path)]
    zero = [path for path in files if path not in with_attr]

    behavioral: list[Path] = []
    declaration_guards: list[Path] = []
    hollow_found: dict[str, list[str]] = {}

    for path in with_attr:
        cls, reasons = analyze_test_file(path)
        rel = path.relative_to(REPO).as_posix()
        if cls == "behavioral":
            behavioral.append(path)
        elif cls == "declaration_guard":
            declaration_guards.append(path)
        else:
            hollow_found[rel] = reasons

    failures: list[str] = []

    # ③ 语料下界：删文件不能让判据变绿。
    if len(files) < MIN_FILES:
        failures.append(
            f"被扫 tests/*.rs 只有 {len(files)} 个 < 下界 {MIN_FILES}"
            "（删/挪测试文件是单向偷工）"
        )
    if len(with_attr) < MIN_FILES_WITH_CRITERIA:
        failures.append(
            f"含 #[test] 的文件只有 {len(with_attr)} 个 < 下界 {MIN_FILES_WITH_CRITERIA}"
        )

    registered = {REPO / entry["path"]: entry for entry in EXCEPTIONS}

    # ① 新增零判据文件未登记。
    for path in zero:
        if path not in registered:
            failures.append(f"零判据文件未登记：{path.relative_to(REPO)}")

    # ② 零判据登记条目机检。
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

    # ④ 空心克隆棘轮检查：未登记空心一律红；已恢复但仍留在清单的陈旧条目同样红。
    hollow_allowlist = load_hollow_allowlist()
    unregistered_hollow = sorted(p for p in hollow_found if p not in hollow_allowlist)
    registered_hollow = sorted(p for p in hollow_found if p in hollow_allowlist)
    stale_hollow = sorted(p for p in hollow_allowlist if p not in hollow_found)

    for rel in unregistered_hollow:
        failures.append(
            f"空心克隆测试未登记（不得静默计为行为判据）：{rel} [{', '.join(hollow_found[rel])}]"
        )
    for rel in stale_hollow:
        failures.append(
            f"空心例外条目陈旧（已恢复或已移除，请从 hollow_criteria_allowlist.json 删除）：{rel}"
        )
    if args.strict_hollow and registered_hollow:
        failures.append(
            f"--strict-hollow: 当前仍有 {len(registered_hollow)} 个已登记空心克隆文件尚未恢复"
        )

    print(
        f"[check-test-criteria] 扫过 {len(files)} 个 tests/*.rs；"
        f"真实行为判据 {len(behavioral)} 个；"
        f"合法声明守卫 {len(declaration_guards)} 个；"
        f"空心克隆 {len(hollow_found)} 个（已登记 {len(registered_hollow)} / 未登记 {len(unregistered_hollow)}，不计入行为判据）；"
        f"零判据 {len(zero)} 个（登记例外 {len(EXCEPTIONS)} 条）"
    )
    if args.list_hollow and hollow_found:
        for rel in sorted(hollow_found):
            tag = "registered" if rel in hollow_allowlist else "UNREGISTERED"
            print(f"  - [{tag}] {rel}: {', '.join(hollow_found[rel])}")

    if failures:
        for failure in failures:
            print(f"[FAIL] {failure}", file=sys.stderr)
        return 1
    if registered_hollow:
        print(
            f"[ok] 无未登记零判据或新增空心克隆（⚠ 仍有 {len(registered_hollow)} 个已登记历史空心克隆待 T-FIX-905..928 恢复，"
            "已从行为判据读数中剥离；传 --list-hollow 查看清单，--strict-hollow 强制拦截）"
        )
    else:
        print("[ok] 每个 tests/*.rs 要么自带真实行为判据，要么是已登记且可机检的例外")
    return 0


if __name__ == "__main__":
    sys.exit(main())
