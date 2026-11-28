#!/usr/bin/env python3
"""Check external URLs in docs/manual-latex/ for reachability.

Usage:
    python3 scripts/gates/check_manual_links.py [--volume arrangement] [--timeout 10] [--workers 16]
"""

from __future__ import annotations

import argparse
import re
import ssl
import sys
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

URL_RE = re.compile(r"\\(?:url|href)\{([^}]+)\}")
UA = "Mozilla/5.0 (compatible; Google-Agent; +https://developers.google.com/crawling/docs/crawlers-fetchers/google-agent)"


def collect_urls(root: Path, volume: str | None) -> dict[str, list[str]]:
    url_map: dict[str, list[str]] = {}
    search_root = root / volume if volume else root
    for tex in sorted(search_root.rglob("*.tex")):
        if tex.name == "ch01-sample.tex":
            continue
        text = tex.read_text(encoding="utf-8")
        for url in URL_RE.findall(text):
            if url.startswith(("http://", "https://")):
                rel = tex.relative_to(root).as_posix()
                url_map.setdefault(url, []).append(rel)
    return url_map


def check_url(url: str, timeout: float) -> tuple[str, int, str]:
    ctx = ssl.create_default_context()
    last_err = ""
    for method in ("HEAD", "GET"):
        req = urllib.request.Request(url, headers={"User-Agent": UA}, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout, context=ctx) as resp:
                return url, int(resp.status), ""
        except urllib.error.HTTPError as exc:
            if method == "HEAD" and exc.code in (400, 403, 405, 501):
                last_err = str(exc)
                continue
            return url, int(exc.code), str(exc)
        except Exception as exc:  # noqa: BLE001
            last_err = str(exc)
            if method == "HEAD":
                continue
            return url, 0, last_err
    return url, 0, last_err or "request_failed"


def main() -> int:
    parser = argparse.ArgumentParser(description="Verify external URLs in docs/manual-latex.")
    parser.add_argument("--root", default="docs/manual-latex", help="Manual LaTeX root directory")
    parser.add_argument("--volume", default=None, help="Optional volume slug (e.g. arrangement)")
    parser.add_argument("--timeout", type=float, default=10.0, help="Per-request timeout seconds")
    parser.add_argument("--workers", type=int, default=16, help="Concurrent worker count")
    args = parser.parse_args()

    root = Path(args.root)
    if not root.is_dir():
        print(f"ERROR: manual root not found: {root}", file=sys.stderr)
        return 2

    url_map = collect_urls(root, args.volume)
    results: list[tuple[str, int, str]] = []
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futs = [pool.submit(check_url, u, args.timeout) for u in sorted(url_map)]
        for fut in as_completed(futs):
            results.append(fut.result())

    results.sort(key=lambda item: (0 if 200 <= item[1] < 400 else 1, item[1], item[0]))
    ok_count = sum(1 for _, code, _ in results if 200 <= code < 400)
    bad = [r for r in results if not (200 <= r[1] < 400)]

    print(f"Checked {len(results)} unique URL(s): {ok_count} OK, {len(bad)} failed.")
    for url, code, err in results:
        status = f"HTTP {code}" if code else f"ERR ({err})"
        refs = ", ".join(url_map[url][:2])
        print(f"  [{status}] {url}  ({refs})")

    if bad:
        print(f"FAILED: {len(bad)} unreachable URL(s) found.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
