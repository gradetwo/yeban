#!/usr/bin/env python3
"""按登记的上游坐标**取回**采样并**逐字节校验**（把"登记式入库"变成"可校验的登记式入库"）。

为什么需要它: `assets/samples/manifest.json` 是**登记式**入库（`items[].optional=true`），
于是门禁的"绿"只证明**清单自洽** —— 实测原文就是 `0/20594 条资产的 SHA-256 与磁盘一致`。
**一个无法取回、无法校验的登记，不是证据，只是清单。** 本脚本用清单里的 `repo` + `pin` + `source_url`
把文件取回来，逐项比对 `sha256` 与 `size_bytes`。

⭐ 任务单 `#201`（负责人原话「下载采样库为啥不是下载各种压缩包啊，这样好慢啊」）:
**每个 instrument 一个 tarball** 是默认模式 —— 清单顶层只有 **30** 个 `instruments`，
而 `items` 有 **20 594** 项。逐文件模式要为每一项发一次 HTTP 请求（20 594 次）；
归档模式每个 instrument 只发一次请求（`https://github.com/<repo>/archive/<pin>.tar.gz`），
**30 次**。校验强度**一点没降**：解包后仍然逐文件比对清单的 `sha256` ＋ `size_bytes`。

用法:
    python3 scripts/dev/fetch-samples.py --limit 20      # 取回**最小**的 20 个文件并校验（有界，CI 手动档用）
    python3 scripts/dev/fetch-samples.py --all           # 全部 20594 个文件（9.4 GiB，只在参考机上做）
    python3 scripts/dev/fetch-samples.py --all --per-file  # ⭐ 回退到逐文件模式（20 594 次请求）
    python3 scripts/dev/fetch-samples.py --check-only    # 只校验磁盘上已有的
    # ⭐ 任务单 #196: 下载到**程序预设的采样库根**（逻辑与 --all 同一份，只是落点不同）
    python3 scripts/dev/fetch-samples.py --all --dest "$HOME/Library/Application Support/dev.yeban.app/Samples"
    # ⭐ 续传：已下过一部分 ⇒ 直接重跑，已校验通过的文件⛔不重下（见下"续传口径"）
    # ⭐ 判据用**本地假源**（⛔ 不联网、⛔ 不写仓库）:
    #   归档模式（默认）的源根布局 = 把 `https://github.com` 换成源根 ⇒
    #     `<源根>/<repo>/archive/<pin>.tar.gz`
    #     （例: /tmp/fake-source/sfzinstruments/aliexpress-erhu/archive/6615….tar.gz）
    #   逐文件模式（--per-file）的源根布局 = 上游文件树原样（见 `upstream_url`）
    python3 scripts/dev/fetch-samples.py --all --manifest /tmp/fake.json --dest /tmp/lib \\
        --source-root file:///tmp/fake-source
退出口径（与 check_vendor.sh / check_release_defaults.sh 同一纪律）:
    0 = 取回的文件**全部**校验通过
    1 = 有字节不符（**含归档本身不可解包**：那是已证实的数据损坏，不是"环境判不了"）
    2 = **无法判定**（离线/被沙箱拦/没有可校验的项）
续传口径（⭐ `#201` 追加: 负责人已经用逐文件模式下了一部分，⛔ 不许让他从头再来）:
    目标文件已存在且 `sha256` ＋ `size_bytes` 与清单一致 ⇒ 记为**已通过**、⛔ 不重下
    （唯一判定函数是 `local_check`，`--check-only` 用的是**同一个**函数）；
    存在但**不符** ⇒ 重新取字节并**替换**（`--check-only` 下只报不符、⛔ 不动磁盘）。
    归档模式下，若某 instrument 的**全部**登记文件都已通过 ⇒ **整个 tarball 都不下**
    （报 `跳过归档:` 与 `跳过 N 个（全部已校验通过）`）。
落点口径:
    --dest 缺省 = 仓库根（`relative_path` 原样，如 `assets/samples/...`）；
    --dest DIR = **库布局**：先剥掉 `assets/samples/` 前缀，文件落到 `DIR/<乐器>/...`
    （与 `crates/yeban-app/src/sfz_load.rs` 的库根候选**同一条**规则）。
只留登记文件（⭐ `#201` 判据④）:
    归档里**未登记**的成员**不落地、也不算失败**，只计数
    （`归档内未登记且未落地的文件 N 个`）；已登记但不在本次范围
    （`--limit` / `--only-ext`）的成员同样不落地。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request

REPO = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = REPO / "assets" / "samples" / "manifest.json"
LIBRARY_PREFIX = "assets/samples/"
# 真实上游的归档端点（`#201`: 每个 instrument 只发这一次请求）。
GITHUB_MIRROR = "https://github.com"
# 流式读文件/归档的块大小（只影响内存占用，不影响读数）。
CHUNK = 1 << 20


# ⚠ **上游剥离表**（临时，直到清单自己登记上游路径）。
#
# 为什么需要: `assets/samples/manifest.json` 的 `relative_path` 是**仓库内**路径，
# 而上游路径是**推导**出来的 —— 两者**不是一一对应**。实测证据（GitHub API `?recursive=1`）:
# `karoryfer.meatbass` 的 pin `ac9e859564bd` 下共 593 个条目，含 `aria_info` 的**只有** `GUI/aria_info.xml`；
# 而清单里的仓库内路径是 `assets/samples/karoryfer-meatbass/Meatbass/GUI/aria_info.xml`
# ⇒ 多出一层 `Meatbass/` ⇒ 推导出的 URL 必然 404。
#
# 语义: 乐器 id -> 从**推导出的上游路径**开头剥离的段数。
# ⚠ 这不是"修好了"：真正的修法是让清单**登记上游路径**（见 `docs/DEVELOPMENT_LEDGER.md` 第 67 轮 needs）。
# 本表只是把已**实测确认**的映射写下来，避免那条 404 继续以「无法判定」的身份占用 `--tolerate-404`。
# ⚠ `#201`：归档模式**共用同一张表**（同一个 `upstream_relative`）—— tarball 里是**真实仓库布局**，
#   所以这里剥离几段，归档里就少几段，两种模式的上游路径口径必然一致。
UPSTREAM_STRIP: dict[str, int] = {
    "karoryfer-meatbass": 1,  # 上游无 `Meatbass/` 这一层（API 实测：唯一命中是 `GUI/aria_info.xml`）
    # 由 `scripts/dev/audit-upstream-paths.py` 全量审计（30 个乐器逐个 HEAD）实测得到:
    "karoryfer-emilyguitar": 1,
}


def _source_base(source_root: str) -> str:
    """源根的 URL 形态（目录 ⇒ `file://` URI；`file://` / `http://` 原样）。"""
    base = source_root
    if "://" not in base:
        base = pathlib.Path(base).resolve().as_uri()
    return base.rstrip("/")


def upstream_relative(instrument: dict, item: dict) -> str:
    """由**仓库内**路径推导**上游**路径（不含 URL 编码、不含主机）。

    `#201` 的关键点：逐文件模式拿它拼 raw URL，归档模式拿它在 tarball 里**找成员** ——
    两种模式共用这一份推导（含 `UPSTREAM_STRIP`），否则两种模式的"上游路径"会漂移。
    """
    relative_root = (instrument.get("relative_root") or instrument.get("prefix") or "").strip("/")
    rel = item.get("relative_path", "")
    # ⚠ `relative_root` **有两种形态**: 有的已是 `assets/samples/vcsl`（**含** `assets/samples/` 前缀），
    # 有的只是 `vcsl`。第一版无条件再拼一次前缀 ⇒ 变成 `assets/samples/assets/samples/vcsl/` ⇒ 全部 404。
    # ⇒ 先归一化，再按**最长匹配**剥离。
    root = relative_root if relative_root.startswith("assets/samples/") else f"assets/samples/{relative_root}"
    prefix = root + "/"
    upstream_path = rel[len(prefix):] if rel.startswith(prefix) else rel
    strip = UPSTREAM_STRIP.get(str(item.get("instrument", "")), 0)
    if strip:
        parts = upstream_path.split("/")
        if len(parts) > strip:
            upstream_path = "/".join(parts[strip:])
    return upstream_path


def upstream_url(instrument: dict, item: dict, source_root: str = "") -> str | None:
    """由 `repo` + `pin` + 相对于乐器根的上游路径拼出 raw URL（**逐文件模式**）。

    `source_root` 非空 ⇒ 用**本地源根**替换 `raw.githubusercontent.com`（判据的假源；
    ⛔ 不联网）。源根可以是目录，也可以是 `file://` URL。
    """
    repo, pin = instrument.get("repo"), instrument.get("pin")
    if source_root:
        repo, pin = repo or "local/mirror", pin or "local"
    if not repo or not pin:
        return None
    # ⚠ 上游路径里有**空格与括号**（实测: "Struck Idiophones/Non-standard pitch (please transpose).txt"）
    # ⇒ 必须按段百分号编码, 否则 http.client 直接抛 InvalidURL（第一版就是这么崩的）。
    encoded = urllib.parse.quote(upstream_relative(instrument, item), safe="/")
    if source_root:
        return _source_base(source_root) + "/" + encoded
    return f"https://raw.githubusercontent.com/{repo}/{pin}/{encoded}"


def archive_url(instrument: dict, source_root: str = "") -> str | None:
    """⭐ `#201` 的归档 URL：`<镜像>/<repo>/archive/<pin>.tar.gz`（每个 instrument 一次请求）。

    `source_root` 非空 ⇒ **整段替换** `https://github.com`（判据的假源；⛔ 不联网）：
    `<源根>/<repo>/archive/<pin>.tar.gz`。这样假源与真源的**路径形状逐字相同**，
    判据量到的请求数就是真实请求数（不是另一套口径）。
    """
    repo, pin = instrument.get("repo"), instrument.get("pin")
    if not repo or not pin:
        return None
    base = _source_base(source_root) if source_root else GITHUB_MIRROR
    return f"{base}/{repo}/archive/{pin}.tar.gz"


def library_relative(item: dict) -> str:
    """`item.relative_path` ⇒ 库布局里的相对路径（剥掉 `assets/samples/`）。"""
    relative = item.get("relative_path", "")
    return relative[len(LIBRARY_PREFIX):] if relative.startswith(LIBRARY_PREFIX) else relative


def target_for(item: dict, dest_root: pathlib.Path | None) -> pathlib.Path:
    """落点：不给 `--dest` ⇒ 仓库内原路径（旧口径）；给 `--dest` ⇒ `<dest>/<乐器>/...`。"""
    if dest_root is not None:
        return dest_root / library_relative(item)
    return REPO / item.get("relative_path", "")


def _within(path: pathlib.Path, root: pathlib.Path) -> bool:
    """`path` 是否在 `root` 之下（⛔ 不许归档成员把字节写到落点外面）。"""
    if path == root:
        return True
    try:
        return path.is_relative_to(root)  # Python ≥ 3.9
    except AttributeError:  # pragma: no cover - 老解释器的兜底
        return str(path).startswith(str(root) + os.sep)


def land_bytes(target: pathlib.Path, data: bytes, root: pathlib.Path) -> None:
    """把**已校验通过**的字节落到 `target`（越界即拒绝）。

    ⚠ 顺序：**先校验、后落地**。旧版逐文件模式是"先写再报不符" ⇒ 坏字节留在磁盘上。
    `#201` 的判据②要求"⛔ 不静默成功"，落地因此只在 sha256 ＋ size 都对上之后发生。
    """
    resolved = pathlib.Path(os.path.normpath(str(target))).resolve()
    if not _within(resolved, root):
        raise ValueError(f"落点越界（拒绝写）: {resolved} 不在 {root} 之下")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    """流式摘要（9 GiB 的库不一次读进内存）。"""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def local_check(target: pathlib.Path, item: dict) -> tuple[str, str, int]:
    """⭐ **"已通过"的唯一判定函数**（续传 ＋ `--check-only` ＋ 归档跳过 共用这一个）。

    返回 `(状态, 实际 sha256, 实际字节数)`，状态 ∈ `{"missing", "ok", "bad"}`。
    ⚠ `#201` 追加判据④要求 `--check-only` 与续传**口径一致** ⇒ 两边都必须调它，
    ⛔ 不许各写一份"文件在不在/对不对"的判断。
    """
    if not target.is_file():
        return ("missing", "", 0)
    digest = sha256_file(target)
    size = target.stat().st_size
    if digest == item.get("sha256") and size == item.get("size_bytes"):
        return ("ok", digest, size)
    return ("bad", digest, size)


class Tally:
    """一次运行的**全部**读数（⛔ 不靠打印语句数数）。"""

    def __init__(self) -> None:
        self.fetched = 0  # 拿到字节的文件数（含不符；= 汇总行的"取回 N"）
        self.verified = 0  # 校验通过的文件数（含续传的已通过项；= "校验通过 N"）
        self.skipped = 0  # 没有上游坐标/本地没有（--check-only）而跳过的项
        self.mismatched = 0  # sha256 或 size 不符
        self.not_found = 0  # 上游 404，或归档里找不到该登记文件
        self.not_found_ids: list[str] = []
        self.resumed = 0  # 磁盘上已通过、⛔ 未重下
        self.unregistered = 0  # 归档里未登记、⛔ 未落地的成员
        self.archives_fetched = 0  # 真正发出的 tarball 请求数
        self.archives_skipped = 0  # 全部已通过 ⇒ 整个 tarball 都没下
        self.archive_errors = 0  # 归档本身打不开/不可解包
        self.offline = False


def report_mismatch(item: dict, digest: str, size: int, tally: Tally) -> None:
    tally.mismatched += 1
    print(
        f"  ✗ {item.get('id')}: sha256/size 不符\n"
        f"      登记 {item.get('sha256')} / {item.get('size_bytes')}\n"
        f"      实际 {digest} / {size}",
        file=sys.stderr,
    )


def verify_local_only(todo: list[dict], dest_root: pathlib.Path | None, tally: Tally) -> None:
    """`--check-only`：只校验磁盘上已有的（⛔ 不联网、⛔ 不动磁盘）。"""
    for item in todo:
        state, digest, size = local_check(target_for(item, dest_root), item)
        if state == "ok":
            tally.verified += 1
        elif state == "bad":
            report_mismatch(item, digest, size, tally)
        else:
            tally.skipped += 1


def fetch_per_file(
    todo: list[dict],
    instruments: dict[str, dict],
    dest_root: pathlib.Path | None,
    source_root: str,
    tally: Tally,
) -> None:
    """逐文件模式：每个文件一次请求（20 594 次）。⛔ 默认不再用，只作 `--per-file` 回退。"""
    root = dest_root if dest_root is not None else REPO
    for item in todo:
        target = target_for(item, dest_root)
        state, digest, size = local_check(target, item)
        if state == "ok":
            # ⭐ 续传：已存在且一致 ⇒ 直接算"已通过"、⛔ 不重下。
            tally.verified += 1
            tally.resumed += 1
            continue
        instrument = instruments.get(item.get("instrument", ""), {})
        url = upstream_url(instrument, item, source_root)
        if url is None:
            tally.skipped += 1
            continue
        try:
            with urllib.request.urlopen(url, timeout=30) as response:  # noqa: S310 (受信坐标来自清单)
                data = response.read()
        except urllib.error.HTTPError:
            # ⚠ **404 ≠ 离线**: 它说明"由清单推导出的上游戏路径不对/该 pin 上没有这个文件" ——
            # 那是**清单或推导口径的问题**（实测: `karoryfer-meatbass/Meatbass/GUI/aria_info.xml` 就是 404），
            # 而**不是**"这个环境判不了"。
            tally.not_found += 1
            tally.not_found_ids.append(item.get("id"))
            continue
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as exc:
            print(f"[unknown] 取回失败（连接/沙箱？）: {item.get('id')} — {exc}", file=sys.stderr)
            tally.offline = True
            continue
        tally.fetched += 1
        digest = sha256_bytes(data)
        if digest == item.get("sha256") and len(data) == item.get("size_bytes"):
            land_bytes(target, data, root)
            tally.verified += 1
        else:
            # 上游这份字节也不对 ⇒ ⛔ 不落地（本地原来那份若是坏的，仍然留在盘点里报不符）。
            report_mismatch(item, digest, len(data), tally)


def _unsafe_member(relative: str) -> bool:
    """归档成员名是否越界（绝对路径 / `..` / 反斜杠盘符）。"""
    if relative.startswith("/") or relative.startswith("\\"):
        return True
    if ":" in relative.split("/")[0]:
        return True
    return any(part in ("..", "") for part in relative.split("/"))


def _stream_archive(
    url: str,
    registered: dict[str, dict],
    scoped: dict[str, dict],
    dest_root: pathlib.Path | None,
    tally: Tally,
    seen: set[str],
) -> None:
    """⭐ 流式解包**一个** instrument 的 tarball：边读边校验，**只落地登记文件**。

    `registered` = 该 instrument **全部**登记项（`上游路径 -> item`），用来区分
    "归档里未登记的文件"（计数、不落地）与"登记了但不在本次范围"
    （`--limit/--only-ext`，静默跳过）。
    `scoped` = 本次**要处理**的项（`上游路径 -> item`，来自 `todo`）。
    """
    root = dest_root if dest_root is not None else REPO
    with urllib.request.urlopen(url, timeout=60) as response:  # noqa: S310 (受信坐标来自清单)
        with tarfile.open(fileobj=response, mode="r|gz") as archive:  # 流式：⛔ 不落临时 tarball
            top = None
            for member in archive:
                name = member.name
                if top is None:
                    # GitHub 的归档**只有一个顶层目录**（`<repo>-<pin>/`）；剥掉它得到仓库内相对路径。
                    # ⚠ 实测（`#201`，真实上游 `freepats/button-accordion-HN`）：这个顶层在 tar 里
                    # **是目录条目、且名字不带尾斜杠**（`…-<pin>`，`isdir=True`）—— 第一版只看
                    # `"/" in name` ⇒ `top=""` ⇒ 一个成员都匹配不上（37/37 全成"未登记"）。
                    # ⇒ 目录条目要自己补 `/`。
                    head = name.rstrip("/")
                    if "/" in head:
                        top = head.split("/", 1)[0] + "/"
                    elif member.isdir():
                        top = head + "/"
                    else:
                        top = ""
                relative = name[len(top):] if top and name.startswith(top) else name
                if not relative or not member.isfile():
                    continue
                if _unsafe_member(relative) or _unsafe_member(name):
                    print(f"  ⚠ 归档里有越界成员，已忽略: {name}", file=sys.stderr)
                    continue
                item = scoped.get(relative)
                if item is None and relative != name:
                    # 兜底：顶层判断一旦猜错（形状不标准），至少还能按**全名**匹配一次。
                    item = scoped.get(name)
                if item is None:
                    if relative not in registered and name not in registered:
                        tally.unregistered += 1
                    continue
                handle = archive.extractfile(member)
                data = handle.read() if handle is not None else b""
                tally.fetched += 1
                digest = sha256_bytes(data)
                seen.add(str(item.get("id")))
                if digest == item.get("sha256") and len(data) == item.get("size_bytes"):
                    land_bytes(target_for(item, dest_root), data, root)
                    tally.verified += 1
                else:
                    report_mismatch(item, digest, len(data), tally)


def fetch_archives(
    todo: list[dict],
    instruments: dict[str, dict],
    registry: dict[str, dict[str, dict]],
    dest_root: pathlib.Path | None,
    source_root: str,
    tally: Tally,
) -> None:
    """⭐ 归档模式（默认）：每个 instrument **一次**请求，解包后逐文件按清单校验。"""
    groups: dict[str, list[dict]] = {}
    for item in todo:
        groups.setdefault(str(item.get("instrument", "")), []).append(item)
    # ⚠ 真实清单里 **2/30** 个 instrument 是 `provenance_form: archive`（`repo`/`pin` 都是 null，
    # `source_url` 指向一个 html 页面）⇒ 没有 tarball 可下，只能计入 `跳过`。
    # ⇒ 上界必须按**有 repo+pin** 的 instrument 数报，⛔ 不能拿 groups 数冒充请求数。
    fetchable = [g for g in groups if archive_url(instruments.get(g, {}), source_root)]
    unreachable = len(groups) - len(fetchable)
    unreachable_files = sum(len(groups[g]) for g in groups if g not in fetchable)
    # "逐文件模式要几次"必须与 `fetch_per_file` **同一口径**：它只为 `upstream_url` 非 None 的
    # 项发请求（没有 repo+pin 且无假源 ⇒ 直接跳过，⛔ 不发请求）。这里直接调它取值，⛔ 不另写一份判断。
    per_file_requests = sum(
        1
        for item in todo
        if upstream_url(instruments.get(str(item.get("instrument", "")), {}), item, source_root)
    )
    print(
        f"[fetch-samples] 归档模式: {len(groups)} 个 instrument / {len(todo)} 个文件 ⇒ "
        f"最多 {len(fetchable)} 次归档请求（逐文件模式要 {per_file_requests} 次）"
        + (
            f"；{unreachable} 个 instrument 没有 repo+pin（archive 型来源）⇒ 跳过 {unreachable_files} 个文件"
            if unreachable
            else ""
        )
    )
    for instrument_id, group in groups.items():
        instrument = instruments.get(instrument_id, {})
        pending: list[dict] = []
        for item in group:
            state, _digest, _size = local_check(target_for(item, dest_root), item)
            if state == "ok":
                tally.verified += 1
                tally.resumed += 1
            else:
                pending.append(item)
        if not pending:
            # ⭐ 续传：这个 instrument 的文件**全部**已校验通过 ⇒ 整个 tarball 都不下。
            tally.archives_skipped += 1
            print(f"[fetch-samples] 跳过归档: {instrument_id}（{len(group)} 个文件全部已校验通过，⛔ 不重下）")
            continue
        url = archive_url(instrument, source_root)
        if url is None:
            tally.skipped += len(pending)
            continue
        print(f"[fetch-samples] 归档: {instrument_id} ← {url}（本次要 {len(pending)}/{len(group)} 个文件）")
        scoped = {upstream_relative(instrument, item): item for item in pending}
        seen: set[str] = set()
        tally.archives_fetched += 1
        try:
            _stream_archive(url, registry.get(instrument_id, {}), scoped, dest_root, tally, seen)
        except urllib.error.HTTPError as exc:
            print(
                f"[fetch-samples] 归档 404: {instrument_id} ← {url}（HTTP {exc.code}）⇒ "
                f"该乐器 {len(pending)} 个文件无法判定",
                file=sys.stderr,
            )
            tally.not_found += len(pending)
            tally.not_found_ids.extend(str(i.get("id")) for i in pending)
            continue
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as exc:
            print(f"[unknown] 归档取回失败（连接/沙箱？）: {instrument_id} — {exc}", file=sys.stderr)
            tally.offline = True
            continue
        except tarfile.TarError as exc:
            # ⚠ 归档**下载到了但打不开** ⇒ 这是**已证实**的数据损坏（不是"环境判不了"）。
            print(f"  ✗ 归档不可解包: {instrument_id} — {exc}", file=sys.stderr)
            tally.archive_errors += 1
            continue
        missing = [item for item in pending if str(item.get("id")) not in seen]
        if missing:
            print(
                f"  ⚠ {instrument_id}: 归档里找不到 {len(missing)} 个登记文件（上游路径推导口径问题？）",
                file=sys.stderr,
            )
            tally.not_found += len(missing)
            tally.not_found_ids.extend(str(i.get("id")) for i in missing)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--only-ext",
        default="",
        help="只取回这些扩展名（逗号分隔，例如 .sfz,.xml,.txt）—— 供人类裁决的「定义与文本类」入库范围使用",
    )
    parser.add_argument("--limit", type=int, default=0, help="最多取回多少个文件（按 size_bytes 升序，取最小）")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--check-only", action="store_true")
    parser.add_argument(
        "--tolerate-404",
        action="store_true",
        help="把上游 404 记为警告而不是失败（临时：清单尚未登记上游路径，见 needs）",
    )
    parser.add_argument(
        "--manifest",
        default=str(MANIFEST),
        help="清单文件（缺省 = 仓库的 assets/samples/manifest.json；判据用本地假清单）",
    )
    parser.add_argument(
        "--dest",
        default="",
        help="落点库根：给了就按库布局写 `<dest>/<乐器>/...`（缺省 = 仓库内原路径）",
    )
    parser.add_argument(
        "--source-root",
        default="",
        help="本地源根（目录或 file:// / http:// URL）：给了就从它取字节，⛔ 不访问 GitHub（判据用）",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--archives",
        dest="mode",
        action="store_const",
        const="archives",
        help="⭐ 归档模式（**默认**）：每个 instrument 一次 `…/archive/<pin>.tar.gz` 请求（30 次）",
    )
    mode.add_argument(
        "--per-file",
        dest="mode",
        action="store_const",
        const="per_file",
        help="逐文件模式（回退）：每个文件一次请求（20 594 次）",
    )
    args = parser.parse_args()
    args.mode = args.mode or "archives"

    manifest = pathlib.Path(args.manifest)
    if not manifest.is_file():
        print(f"没有清单文件: {manifest}", file=sys.stderr)
        return 2
    doc = json.loads(manifest.read_text(encoding="utf-8"))
    instruments = {i["id"]: i for i in doc.get("instruments", [])}
    items = doc.get("items", [])
    if not items:
        print("[unknown] 清单里没有条目", file=sys.stderr)
        return 2
    dest_root = pathlib.Path(args.dest).resolve() if args.dest else None
    if dest_root is not None:
        print(f"[fetch-samples] 落点库根: {dest_root}（库布局，剥掉 {LIBRARY_PREFIX}）")
    if args.source_root:
        print(f"[fetch-samples] 源根: {args.source_root}（⛔ 不访问 GitHub）")
    if args.check_only:
        print("[fetch-samples] 档位: --check-only（只校验磁盘上已有的；⛔ 不取回、⛔ 不替换）")
    elif args.mode == "archives":
        print("[fetch-samples] 档位: 归档模式（每个 instrument 一个 tarball，⭐ 默认）")
    else:
        print("[fetch-samples] 档位: 逐文件模式（每个文件一次请求）")

    tally = Tally()
    # "磁盘上原有 N 项"：只数存在性（⛔ 不在这里哈希 9 GiB；哈希只对本次 todo 做）。
    present = [item for item in items if target_for(item, dest_root).is_file()]

    todo = items if args.all else sorted(items, key=lambda i: i.get("size_bytes", 0))
    if args.only_ext:
        exts = tuple(e.strip().lower() for e in args.only_ext.split(",") if e.strip())
        before = len(todo)
        todo = [i for i in todo if str(i.get("relative_path", "")).lower().endswith(exts)]
        print(f"only-ext 过滤: {before} -> {len(todo)} 条（{args.only_ext}）")
    if args.limit:
        todo = todo[: args.limit]
    if args.check_only:
        # ⚠ `--check-only` 的既有口径（#201 未改）：只看**磁盘上已有**的项，⛔ 与 `--limit` 叠加。
        todo = [item for item in todo if target_for(item, dest_root).is_file()]

    if args.check_only:
        verify_local_only(todo, dest_root, tally)
    elif args.mode == "archives":
        registry: dict[str, dict[str, dict]] = {}
        for item in items:
            instrument_id = str(item.get("instrument", ""))
            instrument = instruments.get(instrument_id, {})
            registry.setdefault(instrument_id, {})[upstream_relative(instrument, item)] = item
        fetch_archives(todo, instruments, registry, dest_root, args.source_root, tally)
    else:
        fetch_per_file(todo, instruments, dest_root, args.source_root, tally)

    print(
        f"fetch-samples: 取回 {tally.fetched} | 校验通过 {tally.verified} | 不符 {tally.mismatched} | "
        f"上游 404（路径推不出来）{tally.not_found} | 跳过 {tally.skipped}"
        f"（清单共 {len(items)} 项，磁盘上原有 {len(present)} 项）"
    )
    if tally.resumed:
        print(f"[fetch-samples] 续传读数: 磁盘上已校验通过 {tally.resumed} 个（⛔ 未重下）")
    if args.mode == "archives" and not args.check_only:
        print(
            f"[fetch-samples] 归档读数: 下载 tarball {tally.archives_fetched} 个｜"
            f"跳过 {tally.archives_skipped} 个（全部已校验通过）｜"
            f"归档内未登记且未落地的文件 {tally.unregistered} 个"
        )
    if tally.not_found_ids:
        # 这是**要修的清单/推导问题**，不是"环境判不了" ⇒ 报出来让人去修，且**不阻断校验通过的部分**。
        print(
            "  ⚠ 以下条目的上游路径推导不出（404 / 归档里找不到）—— 清单没有登记上游路径，"
            "上游路径是**推导**的，这些条目需要人工核对该 pin 上的真实路径:",
            file=sys.stderr,
        )
        for ident in tally.not_found_ids[:10]:
            print(f"      - {ident}", file=sys.stderr)
    if tally.mismatched:
        return 1
    if tally.archive_errors:
        # ⚠ 归档打不开 = **已证实**的数据损坏（pin 不对/传输坏了）⇒ 按 `1` 报，⛔ 不当"无法判定"。
        print(
            f"[unknown] {tally.archive_errors} 个归档不可解包（已证实的数据损坏，⛔ 不是环境问题）",
            file=sys.stderr,
        )
        return 1
    if tally.verified == 0 and tally.offline:
        print("[unknown] 一个文件都没校验到且出现连接错误（离线/沙箱？）", file=sys.stderr)
        return 2
    if tally.not_found:
        # 404 = 清单/推导口径的问题，要人修；但它不该把「其余 N 条已通过」一起判死。
        # ⚠ 默认仍记 2（无法判定）；--tolerate-404 是临时开关（清单尚未登记上游路径期间），
        # 且必须打警告并点名，否则等于把缺陷藏起来 —— 那就成了我在别处反复禁止的假绿。
        if args.tolerate_404:
            print(
                f"::warning::有 {tally.not_found} 条因 404 无法校验（清单未登记上游路径，见 needs）; "
                f"其余 {tally.verified} 条已通过",
                file=sys.stderr,
            )
        else:
            print(
                f"[unknown] 有 {tally.not_found} 条因 404 无法校验（见上）; 其余 {tally.verified} 条已通过",
                file=sys.stderr,
            )
            return 2
    if tally.offline:
        print(f"[unknown] 出现连接错误，已校验 {tally.verified} 个（不是全部）", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
