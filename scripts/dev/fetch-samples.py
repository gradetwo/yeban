#!/usr/bin/env python3
"""按登记的上游坐标**取回**采样并**逐字节校验**（把"登记式入库"变成"可校验的登记式入库"）。

为什么需要它: `assets/samples/manifest.json` 是**登记式**入库（`items[].optional=true`），
于是门禁的"绿"只证明**清单自洽** —— 实测原文就是 `0/20594 条资产的 SHA-256 与磁盘一致`。
**一个无法取回、无法校验的登记，不是证据，只是清单。** 本脚本用清单里的 `repo` + `pin` + `source_url`
把文件取回来，逐项比对 `sha256` 与 `size_bytes`。

用法:
    python3 scripts/dev/fetch-samples.py --limit 20      # 取回**最小**的 20 个文件并校验（有界，CI 手动档用）
    python3 scripts/dev/fetch-samples.py --all           # 全部 20594 个文件（9.4 GiB，只在参考机上做）
    python3 scripts/dev/fetch-samples.py --check-only    # 只校验磁盘上已有的
退出口径（与 check_vendor.sh / check_release_defaults.sh 同一纪律）:
    0 = 取回的文件**全部**校验通过; 1 = 有字节不符; 2 = **无法判定**（离线/被沙箱拦/没有可校验的项）
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
import urllib.error
import urllib.parse
import urllib.request

REPO = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = REPO / "assets" / "samples" / "manifest.json"


def upstream_url(instrument: dict, item: dict) -> str | None:
    """由 `repo` + `pin` + 相对于乐器根的上游路径拼出 raw URL。"""
    repo, pin = instrument.get("repo"), instrument.get("pin")
    if not repo or not pin:
        return None
    relative_root = (instrument.get("relative_root") or instrument.get("prefix") or "").strip("/")
    rel = item.get("relative_path", "")
    # ⚠ `relative_root` **有两种形态**: 有的已是 `assets/samples/vcsl`（**含** `assets/samples/` 前缀），
    # 有的只是 `vcsl`。第一版无条件再拼一次前缀 ⇒ 变成 `assets/samples/assets/samples/vcsl/` ⇒ 全部 404。
    # ⇒ 先归一化，再按**最长匹配**剥离。
    root = relative_root if relative_root.startswith("assets/samples/") else f"assets/samples/{relative_root}"
    prefix = root + "/"
    upstream_path = rel[len(prefix):] if rel.startswith(prefix) else rel
    # ⚠ 上游路径里有**空格与括号**（实测: "Struck Idiophones/Non-standard pitch (please transpose).txt"）
    # ⇒ 必须按段百分号编码, 否则 http.client 直接抛 InvalidURL（第一版就是这么崩的）。
    encoded = urllib.parse.quote(upstream_path, safe="/")
    return f"https://raw.githubusercontent.com/{repo}/{pin}/{encoded}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--limit", type=int, default=0, help="最多取回多少个文件（按 size_bytes 升序，取最小）")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()

    if not MANIFEST.is_file():
        print("没有 assets/samples/manifest.json", file=sys.stderr)
        return 2
    doc = json.loads(MANIFEST.read_text(encoding="utf-8"))
    instruments = {i["id"]: i for i in doc.get("instruments", [])}
    items = doc.get("items", [])
    if not items:
        print("[unknown] 清单里没有条目", file=sys.stderr)
        return 2

    present: list[dict] = []
    for item in items:
        target = REPO / item.get("relative_path", "")
        if target.is_file():
            present.append(item)

    todo = items if args.all else sorted(items, key=lambda i: i.get("size_bytes", 0))
    if args.limit:
        todo = todo[: args.limit]
    if args.check_only:
        todo = present

    fetched = verified = skipped = mismatched = 0
    offline = False
    for item in todo:
        target = REPO / item.get("relative_path", "")
        instrument = instruments.get(item.get("instrument", ""), {})
        if target.is_file():
            data = target.read_bytes()
        else:
            if args.check_only:
                skipped += 1
                continue
            url = upstream_url(instrument, item)
            if url is None:
                skipped += 1
                continue
            try:
                with urllib.request.urlopen(url, timeout=30) as response:  # noqa: S310 (受信坐标来自清单)
                    data = response.read()
            except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError, ValueError) as exc:
                print(f"[unknown] 取回失败（网络/沙箱？）: {item.get('id')} — {exc}", file=sys.stderr)
                offline = True
                break
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            fetched += 1

        digest = hashlib.sha256(data).hexdigest()
        if digest == item.get("sha256") and len(data) == item.get("size_bytes"):
            verified += 1
        else:
            mismatched += 1
            print(
                f"  ✗ {item.get('id')}: sha256/size 不符\n"
                f"      登记 {item.get('sha256')} / {item.get('size_bytes')}\n"
                f"      实际 {digest} / {len(data)}",
                file=sys.stderr,
            )

    print(
        f"fetch-samples: 取回 {fetched} | 校验通过 {verified} | 不符 {mismatched} | 跳过 {skipped}"
        f"（清单共 {len(items)} 项，磁盘上原有 {len(present)} 项）"
    )
    if mismatched:
        return 1
    if verified == 0:
        print("[unknown] 一个文件都没校验到（离线？还是清单里没有可取回的坐标？）", file=sys.stderr)
        return 2
    if offline:
        print(f"[unknown] 网络中断，已校验 {verified} 个（不是全部）", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
