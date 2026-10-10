#!/usr/bin/env python3
"""Three-stage literal-edit applier for `crates/yeban-theory` (my line).

Why: ad-hoc "verify one edit, write it immediately" loops are PARTIAL-APPLY class --
the earlier edits are already on disk when a later anchor fails (really happened in
batches 15 and 21). This entry point makes the discipline mechanical:

  stage 1  verify EVERY anchor (exact literal, exactly one occurrence); any failure
           => print which anchor and how many matches, and WRITE NOTHING (exit 1)
  stage 2  write all files once (only after stage 1 is clean)
  stage 3  read every edit back (the new text present, the old text gone); report
           per-edit mismatches (exit 2 if any) -- catches a partial WRITE

Usage:
  apply_edits.py --edits EDITS.json [--root DIR]
  apply_edits.py --selftest            # known-green feed + known-red feed (positive control)

EDITS.json: [{"file": "<path relative to root>", "old": "<literal>", "new": "<literal>"}, ...]
"""
import argparse
import json
import os
import shutil
import sys
import tempfile

def verify(root, edits):
    """stage 1: return a list of (index, file, occurrences) for anchors that are not unique."""
    bad = []
    for index, edit in enumerate(edits):
        path = os.path.join(root, edit["file"])
        try:
            text = open(path, encoding="utf-8").read()
        except OSError as err:
            bad.append((index, edit["file"], f"unreadable: {err}"))
            continue
        count = text.count(edit["old"])
        if count != 1:
            bad.append((index, edit["file"], count))
    return bad

def apply(root, edits):
    """stage 2: write every file once (grouped per file, preserving order)."""
    by_file = {}
    for edit in edits:
        by_file.setdefault(edit["file"], []).append(edit)
    for name, group in by_file.items():
        path = os.path.join(root, name)
        text = open(path, encoding="utf-8").read()
        for edit in group:
            text = text.replace(edit["old"], edit["new"], 1)
        open(path, "w", encoding="utf-8").write(text)

def read_back(root, edits, before):
    """stage 3: compare COUNTS before/after (a MOVE keeps the text, so plain presence is wrong).

    For every edit: the `old` count must have decreased by exactly one, and -- unless the
    replacement is empty -- the file must contain the `new` text. Counting makes the rule
    correct for insertions, deletions and moves alike (R243(3): no hard-coded expectation).
    """
    bad = []
    for index, edit in enumerate(edits):
        text = open(os.path.join(root, edit["file"]), encoding="utf-8").read()
        # expected delta: -1 for the removed occurrence, +1 when the replacement itself
        # contains the old text (insert-before / copy patterns). Fully computed.
        expected = before[(index, "old")] - 1 + (1 if edit["old"] in edit["new"] else 0)
        if text.count(edit["old"]) != expected:
            bad.append(index)
        elif edit["new"] and edit["new"] not in text:
            bad.append(index)
    return bad

def run(root, edits, label):
    before = {}
    for index, edit in enumerate(edits):
        text = open(os.path.join(root, edit["file"]), encoding="utf-8").read()
        before[(index, "old")] = text.count(edit["old"])
    print(f"[{label}] stage 1: verifying {len(edits)} anchor(s)")
    bad = verify(root, edits)
    if bad:
        for index, name, info in bad:
            print(f"  ANCHOR FAILED edit#{index} file={name} occurrences={info}")
        print(f"[{label}] REFUSED: nothing written ({len(bad)} bad anchor(s))")
        return 1
    print(f"[{label}] stage 2: writing {len(edits)} edit(s)")
    apply(root, edits)
    print(f"[{label}] stage 3: reading back")
    missed = read_back(root, edits, before)
    if missed:
        print(f"[{label}] READ-BACK FAILED for edits {missed}")
        return 2
    print(f"[{label}] OK: all {len(edits)} edit(s) applied and read back")
    return 0

def selftest():
    """Positive control: a known-green feed must apply, a known-red feed must write NOTHING."""
    base = tempfile.mkdtemp(prefix="apply-edits-selftest-")
    try:
        os.makedirs(os.path.join(base, "sub"))
        for name in ("a.rs", "sub/b.rs"):
            open(os.path.join(base, name), "w", encoding="utf-8").write("alpha\nbeta\ngamma\n")
        green = [
            {"file": "a.rs", "old": "alpha", "new": "ALPHA"},
            {"file": "sub/b.rs", "old": "beta", "new": "BETA"},
        ]
        rc_green = run(base, green, "selftest-green")
        green_ok = (rc_green == 0
                    and open(os.path.join(base, "a.rs")).read().startswith("ALPHA")
                    and "BETA" in open(os.path.join(base, "sub/b.rs")).read())
        # known-red: the THIRD anchor is deliberately broken; the first two are valid.
        red = [
            {"file": "a.rs", "old": "gamma", "new": "GAMMA"},
            {"file": "sub/b.rs", "old": "beta", "new": "BETA2"},
            {"file": "sub/b.rs", "old": "NOT-PRESENT", "new": "x"},
        ]
        snapshot = {n: open(os.path.join(base, n)).read() for n in ("a.rs", "sub/b.rs")}
        rc_red = run(base, red, "selftest-red")
        untouched = all(open(os.path.join(base, n)).read() == snapshot[n] for n in snapshot)
        red_ok = (rc_red == 1 and untouched)
        print(f"[selftest] green feed: rc={rc_green} applied={green_ok}")
        print(f"[selftest] red   feed: rc={rc_red} nothing-written={untouched}")
        print("[selftest] OK" if (green_ok and red_ok) else "[selftest] FAILED")
        return 0 if (green_ok and red_ok) else 3
    finally:
        shutil.rmtree(base, ignore_errors=True)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--edits")
    parser.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    if not args.edits:
        parser.error("either --edits FILE.json or --selftest is required")
    return run(args.root, json.load(open(args.edits, encoding="utf-8")), "apply-edits")

if __name__ == "__main__":
    sys.exit(main())
