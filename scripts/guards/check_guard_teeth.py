#!/usr/bin/env python3
"""s37：守卫的**牙齿**自检 —— 对故意做坏输入必须报红 [AGENTS.md §6.3 / 台账 §22.6]。

## 先说指标（这个数字在数什么）
数的是**变异规格**的条数。一条规格 = （被检守卫, 一处对暂存副本的确定性破坏）。
一条规格算**过**当且仅当两半同时成立：
  · 未变异的暂存副本里，该守卫退 0（控件）；
  · 破坏之后，同一守卫退**非** 0（牙齿）。
⚠ 只跑破坏版会放过一类假绿：守卫在副本里根本跑不起来（缺依赖、路径不对）同样退非 0，
  看着像"有牙"，其实是"没牙也没跑"。控件那一半就是把这条路堵死的。

## 为什么存在
`policy_check.py --self-test` 的帮助文本明写它只检查"守卫是否齐备且可调用"，**不构造坏输入**。
于是本仓库的防线一直只有半条：新守卫进了 CI（§22.6 要求的）⇒ 证明它对**当前好输入**绿；
它是否对**坏输入**红，历来靠人手工试一次，试过一次就腐烂。两条实测就在近期：
  · s36 的词表守卫第一版把"下界"判在"少了哪一格"之前 ⇒ 契约被删一格时报的是"被掏空了？"，
    红字失去名字（手工试出来的，见台账 §22.34）；
  · `check_verify_teeth.py` 的整个存在理由就是"有人把 `sys.exit(1)` 删掉、e2e 腿静默变绿"。
本脚本把"手工试一次"变成常驻步骤。

## 名册的防掏空（⛔ 只写下界）
被检名册 = 暂存副本里 `scripts/guards/check_*.py` 的**实际文件集**，与本脚本 MUTANTS 的守卫列
**集合相等**：新增守卫却不给它变异规格 ⇒ 红；规格里写着一个已改名/已删除的守卫 ⇒ 红。
唯一例外是本文件自己（`SELF`，见常量注释）。

## ⚠ 已知不覆盖（登记，不补齐）
1. **只判"红不红"，不判"红得对不对"**：某条规格可能因为另一个原因报红（例如两侧成对删一格时，
   先撞上集合差而不是下界）。要钉住"报的是哪句"就得把每条红字文案抄进本脚本 —— 那是第三份手抄，
   比现状更糟。所以"红得对但理由不对"这一层留给**人读日志**。
2. **不覆盖 `policy_check.py`**：它的 G01–G20 里有读取 `git` 历史的分支（`policy_check.py` 的
   `subprocess.run` 那几处），暂存副本刻意不含 `.git` ⇒ 控件会在"与牙齿无关"的理由上红。
   它需要的机制（带 git 的暂存）是另一票，登记在此不代答。
3. **不覆盖 `check_workflow_structure.py` 的本地运行**：该守卫 `import yaml`，本机没装就退非 0，
   于是它的控件在本地红、在 CI 绿（CI 的 `checks` 腿本来就跑它，依赖在那条腿上装齐）。
   这条**如实报成本地的一条红**，⛔ 不豁免、不静默跳过。
"""
from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GUARDS_DIR = "scripts/guards"
#: 本文件自己：名册若把自检脚本也算进去，就要给它一条"破坏自己"的规格 ⇒ 无限递归。
#: 它由 CI 步骤直接调用（红即整条腿红），不需要变异规格来证明有牙。
SELF = Path(__file__).name

#: 暂存时排除的目录名：`.git`（工作树里是文件，copytree 的 ignore 按名字一样生效）、
#: 构建产物、以及别的线的树。单位 = 排除后的树（本仓库实测 102 MB 量级）。
STAGE_EXCLUDES = {".git", "target", ".worktrees", "node_modules"}


def fail(msg: str) -> None:
    print(f"[红] {msg}", file=sys.stderr)
    sys.exit(1)


# ---------------------------------------------------------------------------
# 变异算子：每个算子先把被改文件的原字节存进 backups，收尾统一还原
# ---------------------------------------------------------------------------

def _backup(stage: Path, rel: str, backups: dict[str, bytes | None]) -> None:
    if rel in backups:
        return
    p = stage / rel
    backups[rel] = p.read_bytes() if p.exists() else None


def sub_once(stage: Path, rel: str, needle: str, repl: str, backups: dict) -> None:
    """把 needle 换成 repl，要求它在目标文件里**恰好出现一次**。"""
    p = stage / rel
    text = p.read_text(encoding="utf-8")
    n = text.count(needle)
    if n != 1:
        fail(f"变异未命中: {rel} 里 {needle!r} 出现 {n} 次（要求恰好 1 次）—— ⛔ 不许空跑着算过")
    _backup(stage, rel, backups)
    p.write_text(text.replace(needle, repl), encoding="utf-8")


def sub_all(stage: Path, rel: str, needle: str, repl: str, backups: dict) -> None:
    """删掉 needle 的**所有**出现（≥1 次），用于"这条判据要求该字样存在"的守卫。"""
    p = stage / rel
    text = p.read_text(encoding="utf-8")
    n = text.count(needle)
    if n < 1:
        fail(f"变异未命中: {rel} 里找不到 {needle!r} —— 被检脚本已改形态，规格需要更新而不是空跑")
    _backup(stage, rel, backups)
    p.write_text(text.replace(needle, repl), encoding="utf-8")


def append_text(stage: Path, rel: str, extra: str, backups: dict) -> None:
    p = stage / rel
    _backup(stage, rel, backups)
    p.write_bytes(p.read_bytes() + extra.encode(encoding="utf-8"))


def unlink(stage: Path, rel: str, backups: dict) -> None:
    p = stage / rel
    if not p.exists():
        fail(f"变异未命中: 暂存副本里没有 {rel}")
    _backup(stage, rel, backups)
    p.unlink()


def pop_json_array(stage: Path, rel: str, pointer: str, backups: dict) -> str:
    """按 `/a/b/c` 形式的键路径取一个 JSON 数组，弹出最后一个元素并回写，返回被弹出的值。

    用 JSON 结构而不是文本字面量：⛔ 手抄一个易腐烂的枚举名当 needle（本仓库的教训）。
    """
    p = stage / rel
    doc = json.loads(p.read_text(encoding="utf-8"))
    node = doc
    keys = [k for k in pointer.split("/") if k]
    if not keys:
        fail(f"变异规格失效: 指针 {pointer!r} 没有键")
    for key in keys:
        if not isinstance(node, dict) or key not in node:
            fail(f"变异规格失效: {rel} 里找不到 {pointer} —— 契约形状变了，规格要跟着改")
        node = node[key]
    if not isinstance(node, list) or not node:
        fail(f"变异规格失效: {pointer} 在 {rel} 里不是非空数组")
    popped = node.pop()
    _backup(stage, rel, backups)
    p.write_text(json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return popped if isinstance(popped, str) else json.dumps(popped)


# ---------------------------------------------------------------------------
# 从被检文件现场推导字面量（⛔ 手抄）
# ---------------------------------------------------------------------------

def first_criteria_name(stage: Path) -> str:
    """从 `check_verify_teeth.py` 自己的 CRITERIA_NAMES 里取第一个名字。"""
    src = (stage / GUARDS_DIR / "check_verify_teeth.py").read_text(encoding="utf-8")
    m = re.search(r"CRITERIA_NAMES\s*=\s*\((.*?)\)", src, re.S)
    if not m:
        fail("推导失效: check_verify_teeth.py 里读不到 CRITERIA_NAMES 元组")
    names = re.findall(r'"([^"]+)"', m.group(1))
    if not names:
        fail("推导失效: CRITERIA_NAMES 是空元组")
    return names[0]


def app_features_last_token(stage: Path) -> str:
    """从 ci.yml 的 `APP_FEATURES: "a,b,c"` 单行里取最后一个特性名。"""
    text = (stage / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    m = re.search(r'^\s*APP_FEATURES:\s*"([^"]+)"', text, re.M)
    if not m:
        fail("推导失效: ci.yml 里读不到 APP_FEATURES 的清单行")
    toks = [t for t in m.group(1).split(",") if t.strip()]
    if len(toks) < 2:
        fail("推导失效: APP_FEATURES 只剩一个特性，无法用它构造「少一项」的变异")
    return toks[-1]


def strip_impl_code(stage: Path, code: str, backups: dict) -> int:
    """把 `"code": "<code>"` 的所有出现从实现侧删掉（只改第一个命中的文件即可满足"两侧成对删"）。"""
    pat = re.compile(r'"code"\s*:\s*"' + re.escape(code) + r'"')
    hits = 0
    for path in sorted((stage / "crates" / "yeban-mcp" / "src").rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        new, n = pat.subn('"code_removed_by_teeth_probe": ""', text)
        if n:
            rel = str(path.relative_to(stage))
            _backup(stage, rel, backups)
            path.write_text(new, encoding="utf-8")
            hits += n
    if not hits:
        fail(f"变异未命中: 实现侧找不到 \"code\": \"{code}\" 的任何构造点")
    return hits


# ---------------------------------------------------------------------------
# 变异规格表
# ---------------------------------------------------------------------------

def mut_legal_changed(stage, bk):
    append_text(stage, "LICENSE", "\n# teeth-probe: 未经人类指示的一行改动\n", bk)


def mut_legal_baseline_gone(stage, bk):
    unlink(stage, f"{GUARDS_DIR}/legal_files.sha256", bk)


def mut_legal_file_gone(stage, bk):
    unlink(stage, "SECURITY.md", bk)


def mut_warning_contract_missing(stage, bk):
    pop_json_array(stage, "schemas/mcp-tools.schema.json", "/definitions/WarningCode/enum", bk)


def mut_warning_paired_delete(stage, bk):
    """两侧成对删同一格：集合等式仍成立，必须靠**防掏空下界**报红。"""
    code = pop_json_array(stage, "schemas/mcp-tools.schema.json", "/definitions/WarningCode/enum", bk)
    strip_impl_code(stage, code, bk)


def mut_verify_name_renamed(stage, bk):
    name = first_criteria_name(stage)
    sub_once(stage, "crates/yeban-mcp/verify/e2e_audio_clip_stdio.sh", f'"{name}"', f'"{name}_renamed"', bk)


def mut_verify_exit_gone(stage, bk):
    sub_all(stage, "crates/yeban-mcp/verify/e2e_audio_clip_stdio.sh", "sys.exit(1)", "# 被探针对删掉", bk)


def mut_workflow_feature_dropped(stage, bk):
    tok = app_features_last_token(stage)
    sub_once(stage, ".github/workflows/ci.yml", f",{tok}\"", "\"", bk)


def mut_vocab_contract_missing(stage, bk):
    pop_json_array(stage, "schemas/project.schema.json",
                   "/properties/tracks/additionalProperties/properties/kind/enum", bk)


def mut_vocab_paired_delete(stage, bk):
    """两侧成对删同一格：TrackKind 少一个变体 + schema 少一格 ⇒ 集合相等，必须靠下界报红。"""
    value = pop_json_array(stage, "schemas/project.schema.json",
                           "/properties/tracks/additionalProperties/properties/kind/enum", bk)
    src = stage / "crates" / "yeban-model" / "src" / "project.rs"
    text = src.read_text(encoding="utf-8")
    m = re.search(r"^pub enum TrackKind \{\s*$", text, re.M)
    if not m:
        fail("推导失效: project.rs 里读不到 `pub enum TrackKind {` 的开头行")
    head = m.end()
    tail = text.index("\n}", head)
    block = text[head:tail]
    line = f"    {value},"
    n = block.count(line)
    if n != 1:
        fail(f"变异未命中: TrackKind 的块里 {line!r} 出现 {n} 次（要求恰好 1 次；"
             f"若该枚举带 #[serde(rename)]，这一格要换成读 serde 映射的另一条规格）")
    i = block.index(line)
    j = i + len(line)
    if block.startswith("\n", j):
        j += 1
    new_block = block[:i] + block[j:]
    _backup(stage, "crates/yeban-model/src/project.rs", bk)
    src.write_text(text[:head] + new_block + text[tail:], encoding="utf-8")


#: 守卫文件 → 它的变异规格 [(规格名, 算子)]
MUTANTS: dict[str, list[tuple[str, object]]] = {
    "check_legal_files_unchanged.py": [
        ("法务文件被改一行", mut_legal_changed),
        ("法务基线被删", mut_legal_baseline_gone),
        ("法务文件缺失", mut_legal_file_gone),
    ],
    "check_warning_codes.py": [
        ("契约少登记一格", mut_warning_contract_missing),
        ("两侧成对删同一格（考下界）", mut_warning_paired_delete),
    ],
    "check_verify_teeth.py": [
        ("判据名字被改名", mut_verify_name_renamed),
        ("非零退出路径被删", mut_verify_exit_gone),
    ],
    "check_workflow_structure.py": [
        ("APP_FEATURES 被删一项", mut_workflow_feature_dropped),
    ],
    "check_project_schema_vocabularies.py": [
        ("契约词表少一格", mut_vocab_contract_missing),
        ("两侧成对删同一格（考下界）", mut_vocab_paired_delete),
    ],
}


# ---------------------------------------------------------------------------
# 驱动
# ---------------------------------------------------------------------------

def run_guard(stage: Path, guard: str) -> tuple[int, str]:
    proc = subprocess.run(
        [sys.executable, str(stage / GUARDS_DIR / guard)],
        cwd=stage, capture_output=True, text=True,
    )
    return proc.returncode, (proc.stdout + proc.stderr)


def restore(stage: Path, backups: dict) -> None:
    for rel, data in backups.items():
        p = stage / rel
        if data is None:
            if p.exists():
                p.unlink()
        else:
            p.write_bytes(data)


def main() -> int:
    if not REPO.joinpath(GUARDS_DIR).is_dir():
        fail(f"找不到守卫目录: {REPO / GUARDS_DIR}")

    stage_root = tempfile.mkdtemp(prefix="guard-teeth-")
    stage = Path(stage_root) / "repo"
    print(f"[读数] 变异规格的单位 = 一条（控件退 0 且破坏后退非 0）；暂存: {stage}")
    shutil.copytree(
        REPO, stage, symlinks=True,
        ignore=shutil.ignore_patterns(*STAGE_EXCLUDES),
    )

    present = {p.name for p in (stage / GUARDS_DIR).glob("check_*.py") if p.name != SELF}
    planned = set(MUTANTS)
    if present != planned:
        fail(
            "守卫名册与本脚本的变异规格**集合不等**：\n"
            f"  有守卫没规格（新增守卫忘了给牙）: {', '.join(sorted(present - planned)) or '（无）'}\n"
            f"  有规格没守卫（名册腐烂或守卫改名）: {', '.join(sorted(planned - present)) or '（无）'}\n"
            f"  实测在位 {len(present)} 枚 / 规格覆盖 {len(planned)} 枚"
        )

    total = passed = 0
    reds: list[str] = []
    for guard in sorted(present):
        rc, out = run_guard(stage, guard)
        if rc != 0:
            first = next((ln for ln in out.splitlines() if ln.strip()), "（无输出）")
            reds.append(f"{guard}: 控件红（未变异就报红，无法归因牙齿）—— 首行: {first}")
            print(f"[控件红] {guard} 在未变异副本上退 {rc}")
            continue
        print(f"[控件绿] {guard}")
        for name, mutate in MUTANTS[guard]:
            total += 1
            backups: dict[str, bytes | None] = {}
            try:
                mutate(stage, backups)
                rc, out = run_guard(stage, guard)
                if rc == 0:
                    reds.append(f"{guard} / {name}: 破坏后仍退 0 —— 这条判据**没有牙**")
                    print(f"[无牙] {guard} / {name}")
                else:
                    passed += 1
                    print(f"[有牙] {guard} / {name}（退 {rc}）")
            finally:
                restore(stage, backups)

    shutil.rmtree(stage_root, ignore_errors=True)

    print(f"[读数] 变异规格 {total} 条，通过 {passed} 条，失败 {total - passed} 条"
          f"（另有 {len([r for r in reds if '控件红' in r])} 枚守卫控件未过）")
    if reds:
        for r in reds:
            print(f"  - {r}", file=sys.stderr)
        return 1
    if total == 0:
        fail("名册为空 —— 没有任何变异规格，本自检不成立")
    print(f"[绿] {passed} 条变异规格全部报红；控件全部退 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
