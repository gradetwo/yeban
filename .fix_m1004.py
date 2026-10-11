import io

p = "/Users/crow/work/music/yeban/docs/ledger/phase-status.md"
s = io.open(p, encoding="utf-8").read()
lines = s.split("\n")

# ---- ① 找到 ROAD-M-1-004 行并翻转状态词 ----
idx = [i for i, ln in enumerate(lines) if ln.startswith("| `ROAD-M-1-004`")]
assert len(idx) == 1, f"ROAD-M-1-004 行数 = {len(idx)}"
i = idx[0]
row = lines[i]
assert "| **部分** |" in row, "该行状态词不是 **部分**"
newrow = row.replace("| **部分** |", "| **已完成** |", 1)

# ⭐ 追加一条就地更正（⛔ 不删旧文：旧文是"当时为什么只到部分"的记录）
correction = (
    " ⭐**【R300 更正（2026-10-11）】**：⭐ 上面那句"
    "「'默认走 WASAPI'… **不是**由一条可判定机制保证的」**已不成立** —— ⭐ **机制在**：⭐ "
    "`scripts/guards/policy_check.py` 的 `g05_forbidden_default_features()`，⭐ 其禁列 "
    "`FORBIDDEN_DEFAULT_FEATURES` **含 `asio`**（⭐ `:59`），⭐ 且已登记进检查表（⭐ `:641`）⇒ "
    "⭐ **每次 `run-gates.sh light` 都跑**（⭐ 实测：⭐ `守卫全部通过 (14 条)`，⭐ 含 G05 与 G07）；⭐ "
    "SDK 那半由 `G07`（`[MUST-GATE-013]`，⭐ `:643`）覆盖；⭐ `AGENTS.md` 红线 6 原文明列 `asio`。⭐ "
    "**所以本项的实质缺件已闭合**；⭐ 剩下的两条是：⭐ ① **RSK-37 的 `unverified` 标记**无处可标** ⇒ "
    "⭐ **条件未到期**（⭐ 标记是为尚不存在的特性准备的，⭐ 而"特性不存在"恰是"默认走 WASAPI"最强的保证）"
    "⇒ ⭐ 已改为**条件式登记**：⭐ **若引入 `asio` 特性，⭐ 必须同时标 `unverified`**；⭐ "
    "② **法务确认（`HD-34`）** ⇒ ⭐ **人类专属**，⭐ 只登记、⛔ 不代签。"
)
if not newrow.rstrip().endswith("|"):
    newrow = newrow.rstrip() + correction
else:
    newrow = newrow.rstrip()[:-1] + correction + " |"
lines[i] = newrow

# ---- ② 更新 Phase -1 的逐阶段汇总 ----
old_sum = "- Phase -1：已完成 2 / 部分 2 / PENDING 2（共 6 项）"
new_sum = "- Phase -1：已完成 3 / 部分 1 / PENDING 2（共 6 项）"
n = s.count(old_sum)
assert n == 1, f"Phase -1 汇总命中 = {n}"
for j, ln in enumerate(lines):
    if ln.strip() == old_sum:
        lines[j] = ln.replace(old_sum, new_sum, 1)
        break

out = "\n".join(lines)
io.open(p, "w", encoding="utf-8").write(out)

# ---- ③ 自证 ----
t = io.open(p, encoding="utf-8").read()
print("状态词已翻转 =", "| `ROAD-M-1-004` |" in t and "**R300 更正" in t)
print("Phase -1 汇总 =", [l for l in t.split("\n") if l.startswith("- Phase -1")])
import collections, re
rows = [l for l in t.split("\n") if l.startswith("| `ROAD-")]
c = collections.Counter()
for l in rows:
    cells = [x.strip() for x in l.split("|")]
    st = cells[3] if len(cells) > 3 else "?"
    if "已完成" in st: c["已完成"] += 1
    elif "部分" in st: c["部分"] += 1
    elif "PENDING" in st: c["PENDING"] += 1
print("逐行统计 =", dict(c), "总 =", len(rows))
