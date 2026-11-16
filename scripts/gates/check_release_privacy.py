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

import json
import re
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
        failures.append("端到端注入: 含 ghp_ 形态 token 的归档应退出 1")
    if dist_rc(b"\x00" + _ATTR_RUN.encode() + b"\x00") != 0:
        failures.append("端到端放行: 属性名拼接的归档应退出 0")

    if failures:
        for failure in failures:
            print(f"[FAIL] --self-test: {failure}", file=sys.stderr)
        return 1
    print("[ok] check_release_privacy --self-test: 注入 9 组 + 放行 2 组 + 附带 2 组 + 端到端 2 组全过")
    return 0


def main(argv: list[str]) -> int:
    dist = Path(argv[1] if len(argv) > 1 else "dist")
    if not dist.is_dir():
        print(f"[unknown] dist 目录缺失: {dist}", file=sys.stderr)
        return 2
    archives = sorted(dist.glob("*.tar.gz"))
    loose = sorted(p for p in dist.iterdir() if p.is_file() and p.suffix != ".gz")
    if not archives and not loose:
        print(f"[unknown] dist/ 里没有任何归档或文件可扫: {dist}", file=sys.stderr)
        return 2

    failures: list[dict[str, str]] = []
    scanned = 0
    for archive in archives:
        try:
            tf = tarfile.open(archive, "r:gz")
        except (tarfile.TarError, OSError) as exc:
            print(f"[unknown] 归档打不开: {archive}: {exc}", file=sys.stderr)
            return 2
        with tf:
            for member in tf.getmembers():
                if not member.isfile():
                    continue
                f = tf.extractfile(member)
                if f is None:
                    print(f"[unknown] 归档成员读不出: {archive}!{member.name}", file=sys.stderr)
                    return 2
                scanned += 1
                for rule in scan_bytes(member.name, f.read()):
                    failures.append({"file": f"{archive.name}!{member.name}", "rule": rule})
    for path in loose:
        scanned += 1
        for rule in scan_bytes(path.name, path.read_bytes()):
            failures.append({"file": path.name, "rule": rule})

    if failures:
        # 只报规则名 + 文件名,绝不报命中值。
        print(json.dumps({"passed": False, "failures": failures}, indent=2))
        return 1
    print(f"[ok] check_release_privacy: 通过 ({len(archives)} 个归档 + {len(loose)} 个散放文件, "
          f"共 {scanned} 个成员, 0 命中)")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        raise SystemExit(_self_test())
    raise SystemExit(main(sys.argv))
