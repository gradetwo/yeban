#!/usr/bin/env python3
"""[AGENTS.md 红线 1] 法务文件基线守卫 —— 未经人类明确指示**不得改动**这六个文件。

逐字依据（AGENTS.md:29）：
  「严禁在未经人类责任人明确书面指示下修改 `LICENSE`、`LEGAL.md`、`SECURITY.md`、
    `TRADEMARK.md`、`GOVERNANCE.md` 与 `NOTICE.md`」

⭐ 为什么需要它（R538/R539）：九条红线里八条有机器守卫，**只有红线 1 没有**。
⚠ 它是**流程规矩**（不是代码属性），故做法是**内容基线**：
   · 文件内容变了 ⇒ 红（提示"若确为人类指示下的合法改动，请一并更新基线"）
   · ⭐ **文件缺失、或基线缺失 ⇒ 也红**（⛔ 不许"没有文件所以没事"这种空转通过）

用法：
    python3 scripts/guards/check_legal_files_unchanged.py            # 校验
    python3 scripts/guards/check_legal_files_unchanged.py --update   # 重新生成基线（须人类指示）
"""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BASELINE = REPO / "scripts" / "guards" / "legal_files.sha256"

# ⭐ 逐字来自 AGENTS.md:29（顺序照抄）
LEGAL_FILES = (
    "LICENSE",
    "LEGAL.md",
    "SECURITY.md",
    "TRADEMARK.md",
    "GOVERNANCE.md",
    "NOTICE.md",
)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_baseline() -> dict[str, str]:
    out: dict[str, str] = {}
    if not BASELINE.exists():
        return out
    for line in BASELINE.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        if len(parts) == 2:
            out[parts[1].strip()] = parts[0]
    return out


def write_baseline() -> int:
    lines = [
        "# [AGENTS.md 红线 1] 法务文件内容基线 —— 仅经人类明确指示才可更新。",
        "# 由 scripts/guards/check_legal_files_unchanged.py --update 生成。",
    ]
    for name in LEGAL_FILES:
        p = REPO / name
        if not p.exists():
            print(f"[FAIL] 基线生成中止：法务文件缺失 {name}", file=sys.stderr)
            return 1
        lines.append(f"{digest(p)}  {name}")
    BASELINE.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"[ok] 已写入基线 {BASELINE.relative_to(REPO)}（{len(LEGAL_FILES)} 个文件）")
    return 0


def self_test() -> int:
    """自检（⭐ 守卫自己也该能被自检 —— 见 R546 的发现：本仓 14 条守卫都没有自检）。

    两向对照：
      · **正对照**：当前工作树与基线一致 ⇒ 必须**不报**
      · **负对照**：把某个文件的摘要改成错的 ⇒ 必须**报出"内容已变"**
    """
    fails = 0

    # ① 基线必须覆盖全部六个文件（否则判据空转）
    base = read_baseline()
    missing = [n for n in LEGAL_FILES if n not in base]
    ok1 = not missing
    print(f"[{'ok' if ok1 else 'FAIL'}] 基线覆盖 {len(LEGAL_FILES)} 个文件（缺 {missing}）")
    fails += 0 if ok1 else 1

    # ② 正对照：真实工作树应当通过
    real = check_current()
    ok2 = not real
    print(f"[{'ok' if ok2 else 'FAIL'}] 正对照：当前工作树报出 {len(real)} 项（须 0）")
    fails += 0 if ok2 else 1

    # ③ 负对照：摘要被篡改必须被抓到
    name = LEGAL_FILES[0]
    p = REPO / name
    if not p.exists():
        print(f"[FAIL] 负对照无法构造：{name} 不存在")
        return 1
    good = digest(p)
    tampered = {**base, name: "0" * 64}
    hit = [n for n in LEGAL_FILES
           if n in tampered and (REPO / n).exists() and digest(REPO / n) != tampered[n]]
    ok3 = hit == [name]
    print(f"[{'ok' if ok3 else 'FAIL'}] 负对照：篡改 {name} 的摘要 ⇒ 抓到 {hit}（须 ['{name}']）")
    fails += 0 if ok3 else 1
    # ⚠ 不改动工作树：上面只比较摘要、不写文件
    assert digest(p) == good, "自检不得改动工作树"

    if fails:
        print(f"[FAIL] 自检 {fails} 项不符 ⇒ 守卫或口径有问题，⛔ 它的绿不算证据")
        return 1
    print("[ok] 自检通过：基线齐备、正对照不误报、负对照能抓到篡改")
    return 0


def check_current() -> list[str]:
    """对**当前工作树**跑一遍红线 1 判据，返回不符项。"""
    bad: list[str] = []
    base = read_baseline()
    for name in LEGAL_FILES:
        if name not in base:
            bad.append(f"基线缺少 {name} ⇒ 判据可能空转，不能当作「红线 1 已满足」的证据")
        if not (REPO / name).exists():
            bad.append(f"法务文件缺失 {name}（红线 1 点名的文件必须在仓库里）")
    for name in LEGAL_FILES:
        p = REPO / name
        if not p.exists() or name not in base:
            continue
        now = digest(p)
        if now != base[name]:
            bad.append(
                f"{name} 内容已变（基线 {base[name][:12]}… 实测 {now[:12]}…）"
                " ⇒ 红线 1 要求人类明确书面指示；若确有指示，请运行 --update 并说明"
            )
    return bad


def main(argv: list[str]) -> int:

    if len(argv) >= 2 and argv[1] == "--self-test":
        return self_test()

    if len(argv) >= 2 and argv[1] == "--update":
        return write_baseline()

    bad: list[str] = check_current()
    base = read_baseline()

    # ⭐ 反空转（照 G06 的做法）：基线必须覆盖全部六个文件，否则本判据不算证据。
    for name in LEGAL_FILES:
        if name not in base:
            bad.append(f"基线缺少 {name} ⇒ 判据可能空转，不能当作「红线 1 已满足」的证据")
        if not (REPO / name).exists():
            bad.append(f"法务文件缺失 {name}（红线 1 点名的文件必须在仓库里）")

    for name in LEGAL_FILES:
        p = REPO / name
        if not p.exists() or name not in base:
            continue
        now = digest(p)
        if now != base[name]:
            bad.append(
                f"{name} 内容已变（基线 {base[name][:12]}… 实测 {now[:12]}…）"
                " ⇒ 红线 1 要求人类明确书面指示；若确有指示，请运行 --update 并说明"
            )

    if bad:
        for b in bad:
            print(f"[FAIL] {b}", file=sys.stderr)
        print(f"[FAIL] 红线 1 守卫：{len(bad)} 项不符", file=sys.stderr)
        return 1
    print(f"[ok] 红线 1 守卫通过：{len(LEGAL_FILES)} 个法务文件与基线一致（基线非空、文件齐备）")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
