#!/usr/bin/env python3
"""跨文件三段写盘（R256②/D1）：① 先校验**全部**锚点 ② 全部通过后**一次写盘** ③ 逐条回读。
用法: apply_edits.py edits.json          输入: [{"file":..., "old":..., "new":...}, ...]
任一锚点失败 ⇒ **一个文件都不写**、非 0 退出、并点名"哪个锚点、几处匹配"。"""
import json, sys, os
if len(sys.argv) != 2:
    print("usage: apply_edits.py edits.json"); sys.exit(2)
edits = json.load(open(sys.argv[1]))
# ---- ① 校验全部锚点（只读）----
plans = []
bad = 0
for i, e in enumerate(edits):
    path = e["file"]
    if not os.path.exists(path):
        print(f"[apply] ✗ #{i} 文件不存在: {path}"); bad += 1; continue
    text = open(path).read()
    hits = text.count(e["old"])
    print(f"[apply] 校验 #{i} {path}: 锚点匹配 {hits} 处（要求恰好 1）")
    if hits != 1:
        print(f"[apply] ✗ #{i} 锚点不合格 ⇒ file={path} matches={hits}"); bad += 1; continue
    plans.append((i, path, text, e["old"], e["new"]))
if bad:
    print(f"[apply] 校验失败 {bad} 处 ⇒ ⛔ 一个文件都不写（非 0 退出）")
    sys.exit(1)
# ---- ② 一次写盘（全部通过后）----
written = []
for i, path, text, old, new in plans:
    open(path, "w").write(text.replace(old, new, 1))
    written.append((i, path, old, new))
    print(f"[apply] 写盘 #{i} {path}")
# ---- ③ 逐条回读 ----
fail = 0
for i, path, old, new in written:
    after = open(path).read()
    if new in after and old not in after:
        print(f"[apply] 回读 ✓ #{i} {path}")
    else:
        print(f"[apply] 回读 ✗ #{i} {path}"); fail += 1
if fail:
    print(f"[apply] 回读失败 {fail} 处"); sys.exit(3)
print(f"[apply] 全部应用并回读通过（文件数 {len(written)}）")
