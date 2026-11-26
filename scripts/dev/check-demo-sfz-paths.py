#!/usr/bin/env python3
"""Demo 容器的 sampler 路径守卫：每条路径必须**真的能解析**。

判据（任务单 T-FIX-575）：

  一条 `tracks.*.sampler.path` 合法 ⇔ 它满足下面**任意一条**：

  1. 是**容器内**的同名 ZIP entry（demo 自带的小样本 / 自包含 SFZ）；
  2. 是 `assets/samples/manifest.json` 里登记的、真实存在的 `.sfz`
     （按 `items[].relative_path` 逐字匹配；本仓清单的登记路径一律以
     `assets/samples/` 开头，因此也接受项目里常见的 `samples/` 前缀写法）。

  两条都不满足 ⇒ **响亮失败**（逐条打印容器 / 轨道 / 原路径），退出码 1。
  容器读不开、`project.json` 缺失或不是合法 JSON ⇒ 同样算失败，**绝不静默跳过**。

用法：
    python3 scripts/dev/check-demo-sfz-paths.py --demos ../yeban-extras/demos
    python3 scripts/dev/check-demo-sfz-paths.py --demos /tmp/demos-before --json
    python3 scripts/dev/check-demo-sfz-paths.py --demos /tmp/demos-after --quiet

退出口径：
    0 = 所有容器的所有 sampler 路径都命中（绿）；
    1 = 至少一条悬空 / 至少一个容器或清单读不开（红）；
    2 = 用法错误（目录不存在 / 清单不存在）。
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import zipfile

REPO = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO / "assets" / "samples" / "manifest.json"
MANIFEST_PREFIX = "assets/samples/"


def load_manifest(path: pathlib.Path) -> tuple[set[str], set[str], int]:
    """返回 (逐字路径集, 去前缀路径集, sfz 登记条数)。"""
    data = json.loads(path.read_text(encoding="utf-8"))
    full: set[str] = set()
    bare: set[str] = set()
    count = 0
    for item in data.get("items", []):
        relative = str(item.get("relative_path", ""))
        if not relative.lower().endswith(".sfz"):
            continue
        # 登记只是一半；真实路径必须**在仓库里存在**，否则"登记的真实 SFZ"仍是空话。
        if not (REPO / relative).is_file():
            continue
        count += 1
        full.add(relative)
        if relative.startswith(MANIFEST_PREFIX):
            bare.add(relative[len(MANIFEST_PREFIX):])
    return full, bare, count


def tracks_of(project: dict) -> list[dict]:
    """工程里的 tracks 在现行 schema 是 `{trackId: Track}`；旧夹具可能是数组。"""
    tracks = project.get("tracks") or {}
    if isinstance(tracks, dict):
        return [track for track in tracks.values() if isinstance(track, dict)]
    if isinstance(tracks, list):
        return [track for track in tracks if isinstance(track, dict)]
    return []


def sampler_path(track: dict) -> str:
    sampler = track.get("sampler")
    if not isinstance(sampler, dict):
        return ""
    return str(sampler.get("path") or "").strip()


def display_name(track: dict) -> str:
    name = track.get("name")
    return str(name) if name else "（未命名）"


def check_container(
    container: pathlib.Path, manifest_full: set[str], manifest_bare: set[str]
) -> tuple[list[str], dict]:
    problems: list[str] = []
    stats = {"container": str(container), "refs": 0, "ok": 0, "red": 0, "tracks": 0}
    try:
        with zipfile.ZipFile(container) as archive:
            names = set(archive.namelist())
            if "project.json" not in names:
                raise KeyError("容器里没有 project.json")
            project = json.loads(archive.read("project.json").decode("utf-8"))
    except (zipfile.BadZipFile, KeyError, UnicodeDecodeError, json.JSONDecodeError) as error:
        problems.append(f"UNREADABLE {container}：{error}")
        stats["red"] += 1
        return problems, stats

    tracks = tracks_of(project)
    stats["tracks"] = len(tracks)
    for track in tracks:
        path = sampler_path(track)
        if not path:
            continue
        stats["refs"] += 1
        normalized = path.replace("\\", "/")
        while normalized.startswith("./"):
            normalized = normalized[2:]
        in_container = normalized in names or path in names
        in_manifest = normalized in manifest_full or normalized in manifest_bare
        if in_container or in_manifest:
            stats["ok"] += 1
            continue
        stats["red"] += 1
        problems.append(
            f"RED {container} | track={track.get('id')} | name={display_name(track)} | path={path}"
        )
    return problems, stats


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--demos", required=True, help="包含若干 .yeban 的 demo 根目录")
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST), help="音源清单（默认仓库内 manifest.json）")
    parser.add_argument("--json", action="store_true", help="最后一行额外打印机器可读读数")
    parser.add_argument("--quiet", action="store_true", help="只打印问题与汇总行")
    args = parser.parse_args(argv)

    demos = pathlib.Path(args.demos)
    manifest_path = pathlib.Path(args.manifest)
    if not demos.is_dir():
        print(f"目录不存在: {demos}", file=sys.stderr)
        return 2
    if not manifest_path.is_file():
        print(f"清单不存在: {manifest_path}", file=sys.stderr)
        return 2

    manifest_full, manifest_bare, sfz_count = load_manifest(manifest_path)
    containers = sorted(demos.rglob("*.yeban"))
    if not containers:
        print(f"目录下没有 .yeban: {demos}", file=sys.stderr)
        return 1

    all_problems: list[str] = []
    stats: list[dict] = []
    for container in containers:
        problems, one = check_container(container, manifest_full, manifest_bare)
        all_problems.extend(problems)
        stats.append(one)
        if not args.quiet:
            state = "GREEN" if not problems else "RED"
            print(
                f"{state} {container} | tracks={one['tracks']} refs={one['refs']} "
                f"ok={one['ok']} red={one['red']}"
            )

    for line in all_problems:
        print(line)

    refs = sum(item["refs"] for item in stats)
    ok = sum(item["ok"] for item in stats)
    red = sum(item["red"] for item in stats)
    print(
        f"check-demo-sfz: demos={len(containers)} tracks={sum(item['tracks'] for item in stats)} "
        f"refs={refs} ok={ok} red={red} manifest_sfz={sfz_count}"
    )
    if args.json:
        print(
            json.dumps(
                {
                    "demos": len(containers),
                    "refs": refs,
                    "ok": ok,
                    "red": red,
                    "problems": all_problems,
                    "containers": stats,
                },
                ensure_ascii=False,
            )
        )
    return 0 if red == 0 and all_problems == [] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
