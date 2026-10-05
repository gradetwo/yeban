#!/usr/bin/env python3
"""抽查每个乐器的**首个条目**能否按其推导路径取回，从而找出"映射可疑"的乐器。

**为什么需要它**（见 `docs/DEVELOPMENT_LEDGER.md` 第 67/68 轮）:
`assets/samples/manifest.json` 的 `relative_path` 是**仓库内**路径，上游路径是**推导**的 ——
两者不保证一一对应（实测 `karoryfer-meatbass` 多一层 `Meatbass/`）。本脚本对**每个乐器**取首个条目做 HEAD，
把结果分成「直接可取回 / 剥离 N 段后可取回 / 仍取不回」三类，**用证据**而非猜测去增长 `UPSTREAM_STRIP`。

退出码: 0 = 所有乐器都属前两类; 1 = 存在"仍取不回"(真缺陷); 2 = 网络不可用(无法判定)。
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import sys
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
RAW = "https://raw.githubusercontent.com/{repo}/{pin}/{path}"
STRIP_CANDIDATES = (0, 1, 2)


def load_fetcher():
    spec = importlib.util.spec_from_file_location("fetch_samples", ROOT / "scripts/dev/fetch-samples.py")
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


def head(url: str, timeout: float = 20.0) -> int:
    req = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "yeban-upstream-audit"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return int(resp.status)
    except urllib.error.HTTPError as exc:
        return int(exc.code)
    except (urllib.error.URLError, TimeoutError, OSError):
        return 0  # 连接层失败 ⇒ 无法判定


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", default="assets/samples/manifest.json")
    ap.add_argument("--only", default="", help="只查这些乐器 id（逗号分隔）")
    args = ap.parse_args()

    doc = json.loads((ROOT / args.manifest).read_text(encoding="utf-8"))
    instruments = {i["id"]: i for i in doc["instruments"]}
    first_item: dict[str, dict] = {}
    for item in doc["items"]:
        first_item.setdefault(item.get("instrument", ""), item)
    wanted = [s for s in args.only.split(",") if s] or sorted(first_item)

    fetcher = load_fetcher()
    direct, stripped, broken, offline = [], [], [], []
    for iid in wanted:
        inst, item = instruments.get(iid), first_item.get(iid)
        if not inst or not item:
            continue
        repo, pin = inst.get("repo"), inst.get("pin")
        rel = item.get("relative_path") or ""
        # ⚠ **口径必须与 `fetch-samples.py` 的 `UPSTREAM_STRIP` 一致**：
        # 先按乐器根（`relative_root`，两种形态都要归一化）剥掉，剩下的才是"上游路径"；
        # `strip=0` 就是取回脚本的默认推导。**第一版只减了 `assets/samples/`、没减乐器根 ⇒ 段数差一**，
        # 于是它对 22 个"其实已正确"的乐器报出"建议剥离 1 段" —— 若照抄进 `UPSTREAM_STRIP`，
        # 会把一个**真实存在**的目录段多剥掉 ⇒ 全部 404。（这类"会给出危险建议的仪器"比没有仪器更糟。）
        root = (inst.get("relative_root") or inst.get("prefix") or "").strip("/")
        root = root if root.startswith("assets/samples/") else f"assets/samples/{root}"
        rel = rel[len(root) + 1:] if rel.startswith(root + "/") else rel.replace("assets/samples/", "", 1)
        parts = rel.split("/")
        codes = []
        for strip in STRIP_CANDIDATES:
            if len(parts) <= strip:
                break
            url = RAW.format(repo=repo, pin=pin, path=urllib.parse.quote("/".join(parts[strip:]), safe="/"))
            code = head(url)
            codes.append(code)
            if code == 200:
                break
        if not codes or codes[0] == 0:
            offline.append(iid)
        elif codes[0] == 200:
            direct.append(iid)
        elif 200 in codes:
            stripped.append((iid, codes.index(200)))
        else:
            broken.append((iid, codes))

    print(f"审计 {len(wanted)} 个乐器（各取首个条目做 HEAD）:")
    print(f"  直接可取回 : {len(direct)}")
    print(f"  剥离后可取回: {len(stripped)}  -> {stripped}")
    print(f"  仍取不回   : {len(broken)}  -> {broken}")
    print(f"  连接失败   : {len(offline)}  -> {offline[:5]}")
    if stripped:
        print("\n建议加入 UPSTREAM_STRIP（段数已按 fetch-samples 的口径归一化；**仅凭这里的实测**）:")
        for iid, n in stripped:
            print(f'    "{iid}": {n},')
    if broken:
        print("\n[失败] 以下乐器的首个条目在 0..2 段剥离内都无法取回（真缺陷，需人工核对上游布局）:", file=sys.stderr)
        return 1
    if offline:
        print(f"\n[unknown] 有 {len(offline)} 个乐器因连接失败无法判定（不是判失败）", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    import urllib.parse  # noqa: E402  (放在此处以免与顶部顺序冲突)
    raise SystemExit(main())
