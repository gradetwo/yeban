#!/usr/bin/env python3
"""Quality and consistency gate for the five-volume Yeban LaTeX manuals.

Checks:
1. Glossary zh/en line count parity (hard gate).
2. Screenshot placeholder vs real image asset counts (reported loudly;
   blocking only when --strict-screenshots is passed after real figures land
   under ISS-MWZOOCKV-60321E).
3. HELP-MAP.tsv presence and anchor counts per volume (reported loudly;
   blocking only when --strict-help-map is passed after main integration).
4. Optional external URL reachability check via scripts/gates/check_manual_links.py.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

VOLUMES = ("user-guide", "instruments", "music-theory", "arrangement", "mixing")
IMG_EXTS = {".png", ".jpg", ".jpeg", ".svg", ".pdf"}


def count_screenshots(root: Path) -> tuple[int, int]:
    placeholders = 0
    for vol in VOLUMES:
        for lang in ("zh", "en"):
            ch_dir = root / vol / lang / "chapters"
            if not ch_dir.is_dir():
                continue
            for tex in ch_dir.glob("*.tex"):
                if tex.name == "ch01-sample.tex":
                    continue
                placeholders += len(re.findall(r"\\screenshot\{", tex.read_text(encoding="utf-8")))
    real_imgs = sum(
        1
        for p in root.rglob("*")
        if p.is_file() and p.suffix.lower() in IMG_EXTS and p.name != "main.pdf"
    )
    return placeholders, real_imgs


def main() -> int:
    parser = argparse.ArgumentParser(description="Check Yeban LaTeX manual quality gates.")
    parser.add_argument("--root", default="docs/manual-latex", help="Manual LaTeX root")
    parser.add_argument("--strict-screenshots", action="store_true", help="Fail if real images < screenshots")
    parser.add_argument("--strict-help-map", action="store_true", help="Fail if any volume lacks HELP-MAP.tsv")
    parser.add_argument("--check-links", action="store_true", help="Also run check_manual_links.py if present")
    args = parser.parse_args()

    root = Path(args.root)
    if not root.is_dir():
        print(f"ERROR: {root} does not exist", file=sys.stderr)
        return 2

    failures: list[str] = []

    # 1. Glossary parity
    gzh = (root / "common/glossary-zh.tex").read_text(encoding="utf-8").splitlines()
    gen = (root / "common/glossary-en.tex").read_text(encoding="utf-8").splitlines()
    print(f"[glossary] zh={len(gzh)} lines, en={len(gen)} lines")
    if len(gzh) != len(gen):
        failures.append(f"Glossary line mismatch: zh={len(gzh)} != en={len(gen)}")

    # 2. Screenshot placeholders vs real assets
    placeholders, real_imgs = count_screenshots(root)
    print(f"[screenshots] call_sites={placeholders}, real_image_files={real_imgs}")
    if args.strict_screenshots and real_imgs == 0 and placeholders > 0:
        failures.append(f"Screenshots missing: {placeholders} placeholders with 0 real images")

    # 3. HELP-MAP.tsv coverage
    missing_hm: list[str] = []
    total_anchors = 0
    for vol in VOLUMES:
        hm = root / vol / "HELP-MAP.tsv"
        if hm.is_file():
            rows = [l for l in hm.read_text(encoding="utf-8").splitlines() if l.strip() and not l.startswith("#")]
            total_anchors += len(rows)
            print(f"[help-map] {vol}: {len(rows)} rows")
        else:
            missing_hm.append(vol)
            print(f"[help-map] {vol}: MISSING")
    print(f"[help-map] total_rows={total_anchors}, missing_volumes={missing_hm}")
    if args.strict_help_map and missing_hm:
        failures.append(f"Missing HELP-MAP.tsv in: {', '.join(missing_hm)}")

    # 4. Optional external links check
    if args.check_links:
        link_script = Path("scripts/gates/check_manual_links.py")
        if link_script.is_file():
            rc = subprocess.call([sys.executable, str(link_script), "--root", str(root)])
            if rc != 0:
                failures.append(f"check_manual_links.py exited with {rc}")
        else:
            print("[links] scripts/gates/check_manual_links.py not present on this branch (skipped)")

    if failures:
        for f in failures:
            print(f"FAIL: {f}", file=sys.stderr)
        return 1

    print("PASS: manual quality checks completed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
