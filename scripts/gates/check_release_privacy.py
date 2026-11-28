#!/usr/bin/env python3
"""发布隐私门:扫描要发布的归档,命中隐私/凭据即失败。

借鉴 director-desk `scripts/check-release-privacy.mjs` 的规则表,按夜半布局重写
(Python 标准库 only,与 scripts/gates 其余 check_*.py 一致)。

只认产物字节,不问源码:扫描 `dist/` 里的 `*.tar.gz`(读成员字节,不落盘解包)+
散放的文本文件(`*.sha256` 等)。与 MUST-GATE-009 同一条纪律——
退出码 0=通过,1=命中(失败),2=无法判定(dist/ 缺失、无归档、归档损坏;
调用方也按红处理,不许把 SKIP 当通过)。
`--self-test` 跑自带牙测(注入必须红、放行必须绿,不读真产物)。
报告只写规则名 + 相对文件名,绝不打印命中的值本身(打印值就是二次泄漏)。

规则(五类):
  1. absolute-windows-path: Windows 盘符绝对路径 —— 只扫文本成员。
     Linux 构建的二进制里可能合法地带有 Windows 路径字面量(跨平台代码),
     家目录形的 Windows 路径仍由规则 2 在二进制里捕获。
  2. user-home-path: 家目录路径 /home/<user>/、/Users/<user>/、C:\\Users\\<user>\\、
     file:///... —— 文本 + 二进制可打印字符串都扫;CI runner 自身的通用账号
     (runner)除外。release 用 debug=1 构建,二进制里嵌的是构建机自己的源码路径,
     排除 runner 前缀是为不把每发必红;开发者的真名 home 路径仍会命中。
  3. private-key: PEM 私钥头。
  4. credential-token: sk-/ghp_/github_pat_/AKIA 等 token 形,及
     api_key/api_secret/access_token/client_secret 的赋值形。token 形带
     「独立成串」纪律:前缀前不得紧贴 token 字母表字符,主体长度有上限且
     其后不得再紧跟同字母表字符——二进制 .rodata 里背靠背存放的字典字面量
     会拼成假 token(实证:usvg 0.48 的 SVG 属性名表中 "mask-size" 的 "sk-"
     加其后 1818 字符的属性名拼接曾被误判,见规则处注释)。真 token 都是
     定长族、独立成串出现,此纪律不放宽判据意图。
  5. bearer-token: Bearer 令牌。
"""

import io
import json
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

# 文本判定:含 NUL 字节即按二进制走可打印字符串路径(对应 LICENSE/BUILD-INFO.txt
# 是纯文本、yeban-app/yeban-mcp 是 ELF 的实际 payload 构成)。
PRINTABLE = re.compile(rb"[\x20-\x7e\t]{6,}")
GENERIC_CI_USERS = {"runner", "build", "github", "actions"}

RULES = [
    (
        "absolute-windows-path",
        # (?<![A-Za-z]) 排除 URL scheme: LICENSE 里的 https://… 的 "s:" 之前是 "p",
        # 盘符 C: 之前是行首/空白/引号。director-desk 的写法要求两段路径, 会漏掉
        # "C:\\file.txt" 这类单段路径; 这里只要求盘符后有一段非空路径。
        re.compile(r"(?<![A-Za-z])[A-Za-z]:[\\/][^\s\"'<>]+"),
        "text-only",
    ),
    (
        "private-key",
        re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----"),
        "all",
    ),
    (
        "credential-token",
        # 「独立成串」纪律(见文件头规则 4):每个分支都带左界(前缀前不得紧贴
        # token 字母表字符)+ 主体上限 + 右界(主体后不得再紧跟同字母表字符)。
        # 缺此纪律的实证误报:usvg 0.48(resvg 依赖)的 SVG 属性名表在 .rodata
        # 里背靠背存放,"mask-size" 的 "sk-" 后紧跟 1818 字符的属性名拼接
        # (unicode-bidi/font-variant-numeric/…,全在 [A-Za-z0-9_-] 内),
        # 被旧规则判成 sk- token(release dry_run 43835643701 隐私门红)。
        # 上限取值:真 token 主体长度——OpenAI 48~160、GitHub 36/82、AWS 恒 16;
        # 252 给足余量且远小于字典拼接的千字符级长串,配合左右界,真 token
        # (独立成串、两侧是引号/空白/行界)照常命中,判据意图不放宽。
        re.compile(
            r"(?<![A-Za-z0-9_])sk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,252}(?![A-Za-z0-9_-])"
            r"|(?<![A-Za-z0-9_])gh[pousr]_[A-Za-z0-9]{20,252}(?![A-Za-z0-9])"
            r"|(?<![A-Za-z0-9_])github_pat_[A-Za-z0-9_]{20,252}(?![A-Za-z0-9_])"
            r"|(?<![A-Za-z0-9])AKIA[0-9A-Z]{16}(?![0-9A-Z])"
        ),
        "all",
    ),
    (
        "credential-assignment",
        re.compile(
            r"(?:api[_-]?key|api[_-]?secret|access[_-]?token|client[_-]?secret)"
            r"\s*[=:]\s*[\"'][^\"'\s]{16,}[\"']",
            re.IGNORECASE,
        ),
        "all",
    ),
    (
        "bearer-token",
        re.compile(r"Bearer\s+[A-Za-z0-9_.\-]{20,}"),
        "all",
    ),
]

# 家目录规则是“带用户名判定”的正则,不能只用一条 pattern:用户名是通用 CI 账号
# (runner)时不算泄漏,其它一律算。
_HOME_POSIX = re.compile(r"(?:/home/|/Users/)([A-Za-z0-9_.\-]{1,64})/")
_HOME_WINDOWS = re.compile(r"[A-Za-z]:\\Users\\([A-Za-z0-9_.\-]{1,64})\\")
_HOME_FILE_URL = re.compile(r"file:///(?:home|Users)/([A-Za-z0-9_.\-]{1,64})/")


def _has_home_hit(text: str) -> bool:
    for rx in (_HOME_POSIX, _HOME_WINDOWS, _HOME_FILE_URL):
        for m in rx.finditer(text):
            if m.group(1) not in GENERIC_CI_USERS:
                return True
    return False


def _strings(data: bytes) -> str:
    return "\n".join(m.group(0).decode("ascii") for m in PRINTABLE.finditer(data))


def scan_bytes(name: str, data: bytes) -> list[str]:
    """扫一份成员字节,返回命中的规则名(去重、保序)。"""
    is_text = b"\x00" not in data
    if is_text:
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            text = data.decode("utf-8", errors="replace")
        scope = "text"
    else:
        text = _strings(data)
        scope = "binary"
    hits: list[str] = []
    for rule, pattern, applies in RULES:
        if applies == "text-only" and scope != "text":
            continue
        if pattern.search(text):
            hits.append(rule)
    if _has_home_hit(text):
        hits.append("user-home-path")
    return hits


# ---------------------------------------------------------------------------
# 牙测(--self-test):注入必须红、放行必须绿。样本 token 均为虚构值,只取
# 真实形态的长度与字符集;放行样本复刻 usvg 属性名表的无分隔拼接形态
# (43835643701 误报的最小复刻)。与 check_mcp_dependency_direction.py 的
# --self-test 同一惯例。
# ---------------------------------------------------------------------------

_FAKE_SK = "sk-" + "Ab1" * 16                   # 主体 48
_FAKE_SK_PROJ = "sk-proj-" + "xY2-" * 30        # proj- 后主体 120
_FAKE_SK_ANT = "sk-ant-api03-" + "zZ9_" * 24    # ant- 后主体 6+96
_FAKE_GHP = "ghp_" + "Q7" * 18                  # 主体 36
_FAKE_PAT = "github_pat_" + "wW8_" * 20 + "wW"  # 主体 82
_FAKE_AKIA = "AKIA" + "IOSFODNN7EXAMPLE"        # AWS 文档公开示例,主体 16

# usvg names.rs 属性名表的形态:字面量在 .rodata 里背靠背、无分隔拼接,
# "mask-size" 的 "sk-" 嵌在长串中段,两侧全是字母/连字符。
_ATTR_RUN = "".join(
    [
        "alignment-baseline", "targetY", "lengthAdjust", "mask-size",
        "unicode-bidi", "font-variant-numeric", "clip-rule", "font-stretch",
        "intercept", "mask-border-slice", "numOctaves", "fill-rule",
        "image-rendering", "surfaceScale", "mix-blend-mode",
        "mask-border-repeat", "line-height", "display", "kerning",
        "shape-subtract", "width", "stroke-miterlimit", "dy",
        "text-decoration-color", "white-space", "diffuseConstant",
        "text-decoration-stroke", "values", "font-size",
        "shape-image-threshold", "mask-image", "unicode-range",
        "baseline-shift", "k3", "text-anchor", "color-rendering",
        "amplitude", "mask-border-width", "stroke-linecap", "paint-order",
        "lighting-color", "dx", "markerWidth", "scale", "color", "in2",
        "targetX", "direction", "pointsAtX", "stitchTiles", "patternUnits",
        "shape-padding", "k2", "font-optical-sizing", "k4", "vector-effect",
        "mask-composite", "stroke-width", "font-variation-settings",
        "mask-border-outset", "stroke-linejoin", "inline-size", "mask-type",
        "filterUnits", "color-profile", "space", "text-decoration-fill",
        "font-kerning", "pointsAtZ", "text-align", "mask-origin",
        "mask-mode", "font-variant-caps", "marker-mid", "shape-rendering",
        "text-rendering", "word-spacing", "mask-clip",
        "font-feature-settings", "radius", "kernelMatrix",
        "mask-border-source", "k1", "opacity", "markerUnits", "visibility",
        "spreadMethod", "pointsAtY", "slope", "tableValues", "order",
        "text-align-last", "rotate", "shape-margin", "limitingConeAngle",
        "font-weight", "text-decoration-line", "requiredExtensions",
        "enable-background", "systemLanguage", "clipPathUnits",
        "stroke-dashoffset", "ry", "class", "mask-border", "text-decoration",
        "startOffset", "mask-position", "writing-mode", "font-synthesis",
        "isolation", "rx", "markerHeight", "stroke-opacity", "height",
        "font-variant-position", "font-family", "dominant-baseline",
        "shape-inside", "letter-spacing", "azimuth", "stdDeviation",
        "flood-color", "flood-opacity", "font-variant-east-asian", "points",
        "text-underline-position", "patternContentUnits", "baseFrequency",
        "color-interpolation", "font-variant-ligatures", "font-style",
        "filter", "text-decoration-style", "preserveAlpha", "divisor",
        "preserveAspectRatio", "pathLength", "marker-start",
        "glyph-orientation-horizontal", "maskUnits", "textLength", "viewBox",
        "text-overflow", "glyph-orientation-vertical", "primitiveUnits",
        "x", "font-variant", "elevation", "color-interpolation-filters",
        "text-indent", "marker-end", "transform-box", "orient",
    ]
)


OPAQUE_COMPRESSED_EXTS = {".dmg", ".msi"}


def _iter_ar_members(data: bytes, source_label: str) -> list[tuple[str, bytes]]:
    """纯标准库解析 Unix ar 归档（.deb 容器），返回 [(member_name, payload), ...]。"""
    if not data.startswith(b"!<arch>\n"):
        raise ValueError(f"{source_label} 不是合法的 ar/.deb 归档")
    pos = 8
    members: list[tuple[str, bytes]] = []
    n = len(data)
    while pos < n:
        if pos + 60 > n:
            raise ValueError(f"{source_label} ar 成员头截断 (offset={pos})")
        hdr = data[pos : pos + 60]
        if hdr[58:60] != b"`\n":
            raise ValueError(f"{source_label} ar 成员魔数非法 (offset={pos})")
        raw_name = hdr[0:16].decode("ascii", errors="replace").strip()
        size_str = hdr[48:58].decode("ascii", errors="replace").strip()
        try:
            size = int(size_str)
        except ValueError as exc:
            raise ValueError(f"{source_label} ar 成员大小非法: {size_str!r}") from exc
        pos += 60
        if size < 0 or pos + size > n:
            raise ValueError(f"{source_label} ar 成员载荷越界: {raw_name}")
        payload = data[pos : pos + size]
        pos += size
        if pos % 2 == 1 and pos < n:
            pos += 1
        name = raw_name.rstrip("/")
        members.append((name, payload))
    return members


def _open_tar_bytes(payload: bytes, member_name: str, deb_path: Path) -> tarfile.TarFile:
    """打开 .deb 内的 control.tar.* / data.tar.*；.zst 尝试用系统 zstd/dpkg-deb 解压，不可用时显式抛错。"""
    if member_name.endswith(".zst"):
        zstd_bin = shutil.which("zstd")
        if zstd_bin:
            proc = subprocess.run(
                [zstd_bin, "-dc"],
                input=payload,
                capture_output=True,
                timeout=60,
            )
            if proc.returncode == 0:
                return tarfile.open(fileobj=io.BytesIO(proc.stdout), mode="r:")
            raise RuntimeError(f"zstd 解压 {member_name} 失败: {proc.stderr.decode(errors='replace').strip()}")
        if member_name.startswith("data.tar") and shutil.which("dpkg-deb"):
            proc = subprocess.run(
                ["dpkg-deb", "--fsys-tarfile", str(deb_path)],
                capture_output=True,
                timeout=60,
            )
            if proc.returncode == 0:
                return tarfile.open(fileobj=io.BytesIO(proc.stdout), mode="r:")
            raise RuntimeError(f"dpkg-deb 解包失败: {proc.stderr.decode(errors='replace').strip()}")
        raise RuntimeError(f"{deb_path.name}!{member_name} 使用 zstd 压缩但本机缺少 zstd/dpkg-deb，无法穿透扫描")
    return tarfile.open(fileobj=io.BytesIO(payload), mode="r:*")


def _scan_tar_stream(
    tf: tarfile.TarFile,
    prefix: str,
    failures: list[dict[str, str]],
) -> int:
    scanned = 0
    for member in tf.getmembers():
        if not member.isfile():
            continue
        f = tf.extractfile(member)
        if f is None:
            raise RuntimeError(f"归档成员读不出: {prefix}!{member.name}")
        scanned += 1
        for rule in scan_bytes(member.name, f.read()):
            failures.append({"file": f"{prefix}!{member.name}", "rule": rule})
    return scanned


def _scan_deb(deb_path: Path, display_name: str, failures: list[dict[str, str]]) -> int:
    """穿透扫描 .deb：解开 ar 容器并逐文件扫描 control.tar.* 与 data.tar.*。"""
    raw = deb_path.read_bytes()
    ar_members = _iter_ar_members(raw, display_name)
    tar_members = [
        (mname, payload)
        for mname, payload in ar_members
        if mname.startswith("data.tar") or mname.startswith("control.tar")
    ]
    if not any(mname.startswith("data.tar") for mname, _ in tar_members):
        raise ValueError(f"{display_name} 缺少 data.tar.* 成员，无法完成穿透扫描")
    scanned = 0
    for mname, payload in tar_members:
        with _open_tar_bytes(payload, mname, deb_path) as tf:
            scanned += _scan_tar_stream(tf, f"{display_name}!{mname}", failures)
    return scanned


def _make_ar_entry(name: str, payload: bytes) -> bytes:
    """构造最小合法 Unix ar 成员（仅供 --self-test 构造真实 .deb 样本）。"""
    header = (
        f"{name:<16}"
        f"{'0':<12}"
        f"{'0':<6}"
        f"{'0':<6}"
        f"{'100644':<8}"
        f"{len(payload):<10}"
        "`\n"
    ).encode("ascii")
    assert len(header) == 60
    pad = b"\n" if len(payload) % 2 == 1 else b""
    return header + payload + pad


def _build_test_deb(data_files: dict[str, bytes]) -> bytes:
    """构造可供 main() 穿透扫描的真实 .deb (ar + control.tar.gz + data.tar.gz)。"""
    ctrl_buf = io.BytesIO()
    with tarfile.open(fileobj=ctrl_buf, mode="w:gz") as tf:
        ctrl = b"Package: yeban\nVersion: 0.0.1\nArchitecture: amd64\n"
        info = tarfile.TarInfo("./control")
        info.size = len(ctrl)
        tf.addfile(info, io.BytesIO(ctrl))
    data_buf = io.BytesIO()
    with tarfile.open(fileobj=data_buf, mode="w:gz") as tf:
        for rel_name, content in data_files.items():
            info = tarfile.TarInfo(rel_name)
            info.size = len(content)
            tf.addfile(info, io.BytesIO(content))
    return (
        b"!<arch>\n"
        + _make_ar_entry("debian-binary", b"2.0\n")
        + _make_ar_entry("control.tar.gz", ctrl_buf.getvalue())
        + _make_ar_entry("data.tar.gz", data_buf.getvalue())
    )


def _self_test() -> int:
    import io
    import tempfile

    failures: list[str] = []

    def expect_hit(name: str, data: bytes, rule: str = "credential-token") -> None:
        if rule not in scan_bytes(name, data):
            failures.append(f"注入未命中: {name} 应命中 {rule}")

    def expect_clean(name: str, data: bytes, rule: str = "credential-token") -> None:
        if rule in scan_bytes(name, data):
            failures.append(f"放行被误命中: {name} 不应命中 {rule}")

    # 注入:文本作用域(无 NUL)——六个 token 族逐个独立成串出现。
    for label, tok in [
        ("sk", _FAKE_SK),
        ("sk-proj", _FAKE_SK_PROJ),
        ("sk-ant", _FAKE_SK_ANT),
        ("ghp", _FAKE_GHP),
        ("github_pat", _FAKE_PAT),
        ("AKIA", _FAKE_AKIA),
    ]:
        expect_hit(f"text-{label}", f'config line: "{tok}"\n'.encode())
    # 注入:二进制作用域(含 NUL,走可打印字符串路径)。
    expect_hit("bin-sk", b"\x00\x02" + _FAKE_SK.encode() + b"\x00\x99")
    expect_hit("bin-ghp", b"\x00\x02" + _FAKE_GHP.encode() + b"\x00\x99")
    expect_hit("bin-pat", b"\x00\x02" + _FAKE_PAT.encode() + b"\x00\x99")
    # 放行:usvg 属性名无分隔拼接(内含 mask-size),文本与二进制各一。
    expect_clean("attr-run-text", _ATTR_RUN.encode())
    expect_clean("attr-run-bin", b"\x00\x07" + _ATTR_RUN.encode() + b"\x00\x00")
    # 附带完整性:未改动的规则仍须有牙。
    expect_hit("pem", b"-----BEGIN PRIVATE KEY-----\n", rule="private-key")
    expect_hit("bearer", b"Authorization: Bearer " + b"A1" * 12 + b"\n", rule="bearer-token")

    # 端到端:临时 dist + tar.gz,走 main() 真入口验退出码。
    def dist_rc(member: bytes) -> int:
        with tempfile.TemporaryDirectory() as td:
            dist = Path(td)
            buf = io.BytesIO()
            with tarfile.open(fileobj=buf, mode="w:gz") as tf:
                info = tarfile.TarInfo("pkg/yeban-app")
                info.size = len(member)
                tf.addfile(info, io.BytesIO(member))
            (dist / "pkg.tar.gz").write_bytes(buf.getvalue())
            return main(["check_release_privacy.py", str(dist)])

    if dist_rc(b"\x00" + _FAKE_GHP.encode() + b"\x00") != 1:
        failures.append("端到端注入: 含 ghp_ 形态 token 的 .tar.gz 应退出 1")
    if dist_rc(b"\x00" + _ATTR_RUN.encode() + b"\x00") != 0:
        failures.append("端到端放行: 属性名拼接的 .tar.gz 应退出 0")

    # 端到端 .deb 穿透扫描（红 + 绿）
    with tempfile.TemporaryDirectory() as td:
        dist = Path(td)
        (dist / "yeban-dirty.deb").write_bytes(
            _build_test_deb({"./usr/bin/yeban-app": b"\x00" + _FAKE_GHP.encode() + b"\x00"})
        )
        if main(["check_release_privacy.py", str(dist)]) != 1:
            failures.append("端到端注入(.deb): data.tar.gz 含敏感串应退出 1")

    with tempfile.TemporaryDirectory() as td:
        dist = Path(td)
        (dist / "yeban-clean.deb").write_bytes(
            _build_test_deb({"./usr/bin/yeban-app": b"\x00" + _ATTR_RUN.encode() + b"\x00"})
        )
        if main(["check_release_privacy.py", str(dist)]) != 0:
            failures.append("端到端放行(.deb): 干净 .deb 应退出 0")

    # 端到端 stage 目录直接扫描（红 + 绿）
    with tempfile.TemporaryDirectory() as td:
        stage = Path(td)
        (stage / "usr" / "share" / "yeban").mkdir(parents=True)
        (stage / "usr" / "share" / "yeban" / "note.txt").write_text(
            "leaked: /Users/alice/work/yeban/src/main.rs\n", encoding="utf-8"
        )
        if main(["check_release_privacy.py", str(stage)]) != 1:
            failures.append("端到端注入(stage): stage 目录含家目录路径应退出 1")

    with tempfile.TemporaryDirectory() as td:
        stage = Path(td)
        (stage / "usr" / "bin").mkdir(parents=True)
        (stage / "usr" / "bin" / "yeban-app").write_bytes(b"\x00" + _ATTR_RUN.encode() + b"\x00")
        if main(["check_release_privacy.py", str(stage)]) != 0:
            failures.append("端到端放行(stage): 干净 stage 目录应退出 0")

    # 端到端 .dmg / .msi 不可穿透压缩格式必须响亮报 unknown(2)，不得假绿
    for ext in (".dmg", ".msi"):
        with tempfile.TemporaryDirectory() as td:
            dist = Path(td)
            (dist / f"yeban-test{ext}").write_bytes(b"compressed-opaque-payload")
            if main(["check_release_privacy.py", str(dist)]) != 2:
                failures.append(f"端到端未覆盖格式({ext}): 不可穿透压缩包应响亮退出 2，不得冒充已扫")

    if failures:
        for failure in failures:
            print(f"[FAIL] --self-test: {failure}", file=sys.stderr)
        return 1
    print("[ok] check_release_privacy --self-test: 注入 9 组 + 放行 2 组 + 附带 2 组 + 端到端 8 组(.tar.gz/.deb/stage/.dmg/.msi)全过")
    return 0


def main(argv: list[str]) -> int:
    raw_args = argv[1:]
    if raw_args and raw_args[0] == "--stage":
        raw_args = raw_args[1:]
    dist = Path(raw_args[0] if raw_args else "dist")
    if not dist.is_dir():
        print(f"[unknown] 扫描目录缺失: {dist}", file=sys.stderr)
        return 2

    all_files = sorted(p for p in dist.rglob("*") if p.is_file())
    if not all_files:
        print(f"[unknown] 扫描目录里没有任何归档或文件可扫: {dist}", file=sys.stderr)
        return 2

    tar_gzs: list[Path] = []
    debs: list[Path] = []
    opaque: list[Path] = []
    loose: list[Path] = []
    for p in all_files:
        lname = p.name.lower()
        if lname.endswith(".tar.gz") or lname.endswith(".tgz"):
            tar_gzs.append(p)
        elif lname.endswith(".deb"):
            debs.append(p)
        elif p.suffix.lower() in OPAQUE_COMPRESSED_EXTS:
            opaque.append(p)
        else:
            loose.append(p)

    failures: list[dict[str, str]] = []
    scanned = 0

    for archive in tar_gzs:
        rel_name = archive.relative_to(dist).as_posix()
        try:
            with tarfile.open(archive, "r:gz") as tf:
                scanned += _scan_tar_stream(tf, rel_name, failures)
        except (tarfile.TarError, OSError, RuntimeError) as exc:
            print(f"[unknown] 归档打不开: {archive}: {exc}", file=sys.stderr)
            return 2

    for deb in debs:
        rel_name = deb.relative_to(dist).as_posix()
        try:
            scanned += _scan_deb(deb, rel_name, failures)
        except (tarfile.TarError, OSError, ValueError, RuntimeError) as exc:
            print(f"[unknown] .deb 穿透扫描失败: {deb}: {exc}", file=sys.stderr)
            return 2

    for path in loose:
        rel_name = path.relative_to(dist).as_posix()
        scanned += 1
        for rule in scan_bytes(rel_name, path.read_bytes()):
            failures.append({"file": rel_name, "rule": rule})

    if failures:
        # 只报规则名 + 文件名,绝不报命中值。
        print(json.dumps({"passed": False, "failures": failures}, indent=2))
        return 1

    if opaque:
        # 不得把对压缩容器外层字节的扫描冒充内部文件已扫。
        names = ", ".join(p.relative_to(dist).as_posix() for p in opaque)
        print(
            f"[unknown] 检测到当前平台无法穿透扫描的压缩安装包: {names}；"
            "请改扫其打包前 stage 目录（支持直接传入 stage 目录路径），禁止将压缩字节扫描视为通过。",
            file=sys.stderr,
        )
        return 2

    print(
        f"[ok] check_release_privacy: 通过 ({len(tar_gzs)} 个 tar.gz + {len(debs)} 个 .deb + "
        f"{len(loose)} 个散放/stage 文件, 共 {scanned} 个成员, 0 命中)"
    )
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        raise SystemExit(_self_test())
    raise SystemExit(main(sys.argv))
