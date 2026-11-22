#!/usr/bin/env python3
"""Measure yeban-mcp (stdio) process cold-start latency.

Usage:
  python3 scripts/gates/measure_mcp_cold_start.py target/debug/yeban-mcp
  python3 scripts/gates/measure_mcp_cold_start.py target/release/yeban-mcp --runs 60

Each run starts a fresh process, sends `initialize`, waits for the first JSON-RPC
response line, and stops the clock.  The reported p50/p95 are per-process warm-page-
cache startup numbers; the first run is also reported separately so a cold disk
cache is not hidden inside an average.
"""
import argparse
import json
import statistics
import subprocess
import sys
import time


def one_run(binary: str) -> float:
    request = (
        b'{"jsonrpc":"2.0","id":1,"method":"initialize"}\n'
        b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'
    )
    started = time.perf_counter()
    proc = subprocess.Popen(
        [binary, "--stdio"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        assert proc.stdin is not None and proc.stdout is not None
        proc.stdin.write(request)
        proc.stdin.flush()
        proc.stdin.close()
        proc.stdout.readline()
    finally:
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    return (time.perf_counter() - started) * 1000.0


def percentile(sorted_values: list[float], fraction: float) -> float:
    index = max(0, min(len(sorted_values) - 1, int(fraction * len(sorted_values)) - 1))
    return sorted_values[index]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("--runs", type=int, default=60)
    args = parser.parse_args()
    if args.runs < 2:
        parser.error("--runs must be at least 2")

    samples = [one_run(args.binary) for _ in range(args.runs)]
    first, rest = samples[0], sorted(samples[1:])
    report = {
        "binary": args.binary,
        "runs": args.runs,
        "unit": "ms",
        "first_run": round(first, 2),
        "p50": round(statistics.median(rest), 2),
        "p95": round(percentile(rest, 0.95), 2),
        "p99": round(percentile(rest, 0.99), 2),
        "max": round(rest[-1], 2),
    }
    print(json.dumps(report, indent=2))
    return 0 if report["p95"] < 20.0 else 1


if __name__ == "__main__":
    sys.exit(main())
