#!/usr/bin/env python3
"""发布隐私门:扫描要发布的归档,命中隐私/凭据即失败。

借鉴 director-desk `scripts/check-release-privacy.mjs` 的规则表,按夜半布局重写
(Python 标准库 only,与 scripts/gates 其余 check_*.py 一致)。

只认产物字节,不问源码:扫描 `dist/` 里的 `*.tar.gz`(读成员字节,不落盘解包)+
散放的文本文件(`*.sha256` 等)。与 MUST-GATE-009 同一条纪律——
退出码 0=通过,1=命中(失败),2=无法判定(dist/ 缺失、无归档、归档损坏;
调用方也按红处理,不许把 SKIP 当通过)。
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
     api_key/api_secret/access_token/client_secret 的赋值形。
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
        re.compile(
            r"(?:sk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}"
            r"|gh[pousr]_[A-Za-z0-9]{20,}"
            r"|github_pat_[A-Za-z0-9_]{20,}"
            r"|AKIA[0-9A-Z]{16})"
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
    raise SystemExit(main(sys.argv))
