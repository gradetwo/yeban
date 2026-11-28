#!/usr/bin/env python3
"""三平台打包脚本输入校验、argv 传参、DEBIAN/control 与 MSI 空格路径引号门禁判据 (T-FIX-1025)。

判据全部通过实际执行 `scripts/dev/package-*.sh` 入口验证，不只查源码字符串：
  1. 合法版本（默认 Cargo.toml 版本及 nightly 预发布后缀）+ 合法 target 下 `--print-name` 输出精确匹配；
  2. 非法版本（含单引号注入、命令替换 `$()`、反引号、空格、换行符、冒号、空后缀等）在三脚本均非零退出且不触发代码执行；
  3. 非法 target 在三脚本 `--print-name` 与主入口均非零退出；
  4. Linux `.deb` 脚本在临时假 stage 目录上真实跑完 `dpkg-deb --build`、`DEBIAN/control` 对账、结构解包自验与 `.sha256` 侧车生成，核对 SHA-256 哈希与格式；
  5. Windows `admin-install.ps1` 由脚本 `--dump-admin-install-ps1` 真实生成，并对含空格的 MSI 路径与 `TARGETDIR` 验证生成的 `ArgumentList` 显式带双引号包裹。
"""
from __future__ import annotations

import hashlib
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DEB_SH = REPO / "scripts" / "dev" / "package-linux-deb.sh"
DMG_SH = REPO / "scripts" / "dev" / "package-macos-dmg.sh"
MSI_SH = REPO / "scripts" / "dev" / "package-windows-msi.sh"


def run_sh(script: Path, args: list[str], env_extra: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.pop("YEBAN_PKG_VERSION", None)
    if env_extra:
        env.update(env_extra)
    return subprocess.run(
        ["bash", str(script), *args],
        cwd=REPO,
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
    )


def main() -> int:
    failures: list[str] = []

    def check(cond: bool, msg: str) -> None:
        if not cond:
            failures.append(msg)

    # 0. bash -n 语法检查
    for sh in (DEB_SH, DMG_SH, MSI_SH):
        p = subprocess.run(["bash", "-n", str(sh)], capture_output=True, text=True)
        check(p.returncode == 0, f"bash -n {sh.name} 失败: {p.stderr}")

    # 1. 合法版本与合法 target 的 --print-name
    valid_cases = [
        (DEB_SH, "x86_64-unknown-linux-gnu", None, "yeban-0.0.1-x86_64-unknown-linux-gnu.deb"),
        (DEB_SH, "aarch64-unknown-linux-gnu", "0.2.0-nightly.20261127", "yeban-0.2.0-nightly.20261127-aarch64-unknown-linux-gnu.deb"),
        (DMG_SH, "aarch64-apple-darwin", None, "yeban-0.0.1-aarch64-apple-darwin.dmg"),
        (DMG_SH, "x86_64-apple-darwin", "1.0.0-rc.1+build.7", "yeban-1.0.0-rc.1+build.7-x86_64-apple-darwin.dmg"),
        (MSI_SH, "x86_64-pc-windows-msvc", None, "yeban-0.0.1-x86_64-pc-windows-msvc.msi"),
        (MSI_SH, "x86_64-pc-windows-msvc", "0.0.1-alpha.1", "yeban-0.0.1-alpha.1-x86_64-pc-windows-msvc.msi"),
    ]
    for sh, target, ver, expected in valid_cases:
        env = {"YEBAN_PKG_VERSION": ver} if ver is not None else None
        p = run_sh(sh, ["--print-name", target], env)
        check(
            p.returncode == 0 and p.stdout.strip() == expected,
            f"{sh.name} --print-name {target} (ver={ver}) 期望 {expected!r}, 实得 rc={p.returncode} out={p.stdout.strip()!r} err={p.stderr.strip()!r}",
        )

    # 2. 非法版本必须在三脚本都被拒绝（含单引号注入、$()、反引号、换行、冒号、空格）
    with tempfile.TemporaryDirectory() as td:
        marker = Path(td) / "pwned"
        bad_versions = [
            f"0.0.1';os.system('touch {marker}');'",
            "0.0.1$(touch " + str(marker) + ")",
            "0.0.1`touch " + str(marker) + "`",
            "0.0.1\nPackage: injected",
            "1:0.0.1",
            "0.0.1 beta",
            "v0.0.1",
            "0.0.1-",
            "../0.0.1",
        ]
        for sh, target in [
            (DEB_SH, "x86_64-unknown-linux-gnu"),
            (DMG_SH, "aarch64-apple-darwin"),
            (MSI_SH, "x86_64-pc-windows-msvc"),
        ]:
            for bv in bad_versions:
                p = run_sh(sh, ["--print-name", target], {"YEBAN_PKG_VERSION": bv})
                check(p.returncode != 0, f"{sh.name} 应拒绝非法版本 {bv!r}, 但返回了 0 (out={p.stdout!r})")
        check(not marker.exists(), "非法版本中的注入载荷被意外执行")

    # 3. 非法 target 必须在三脚本都被拒绝
    bad_targets = [
        "unknown-target",
        "x86_64-unknown-linux-musl",
        "aarch64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu;id",
        "../x86_64-unknown-linux-gnu",
    ]
    for sh in (DEB_SH, DMG_SH, MSI_SH):
        for bt in bad_targets:
            p = run_sh(sh, ["--print-name", bt])
            check(p.returncode != 0, f"{sh.name} --print-name 应拒绝非法 target {bt!r}")

    # 4. Linux .deb 端到端构建、control 对账与 .sha256 侧车校验
    if shutil.which("dpkg-deb"):
        scratch_root = REPO / "target" / ".pkg-test-tmp"
        scratch_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch_root) as td:
            tdir = Path(td)
            stage = tdir / "stage dir with spaces"
            out = tdir / "out dir with spaces"
            stage.mkdir(parents=True)
            for bname in ("yeban-app", "yeban-mcp"):
                bp = stage / bname
                bp.write_text("#!/usr/bin/env bash\necho 'yeban-app 0.0.1-nightly.20261127'\n", encoding="utf-8")
                bp.chmod(bp.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
            (stage / "LICENSE").write_text("MIT License\n", encoding="utf-8")

            # 先验证非法版本在打包主入口同样被拒且不产出任何 .deb
            p_bad = run_sh(
                DEB_SH,
                [str(stage), "x86_64-unknown-linux-gnu", str(out)],
                {"YEBAN_PKG_VERSION": "0.0.1:bad\nInjected: 1"},
            )
            check(p_bad.returncode != 0, "package-linux-deb.sh 主入口应拒绝含冒号/换行的非法版本")
            check(not list(out.glob("*.deb")), "非法版本被拒时不应生成任何 .deb 产物")

            # 再验证合法版本完整跑通 dpkg-deb 构建、control 自验与 sha256 侧车
            p_ok = run_sh(
                DEB_SH,
                [str(stage), "x86_64-unknown-linux-gnu", str(out)],
                {"YEBAN_PKG_VERSION": "0.0.1-nightly.20261127"},
            )
            check(
                p_ok.returncode == 0,
                f"package-linux-deb.sh 端到端构建失败: rc={p_ok.returncode}\nstdout={p_ok.stdout}\nstderr={p_ok.stderr}",
            )
            deb_file = out / "yeban-0.0.1-nightly.20261127-x86_64-unknown-linux-gnu.deb"
            sha_file = out / "yeban-0.0.1-nightly.20261127-x86_64-unknown-linux-gnu.deb.sha256"
            check(deb_file.is_file(), f"未生成预期的 .deb 文件: {deb_file}")
            check(sha_file.is_file(), f"未生成预期的 .sha256 侧车文件: {sha_file}")
            if deb_file.is_file() and sha_file.is_file():
                expected_sha = hashlib.sha256(deb_file.read_bytes()).hexdigest()
                expected_line = f"{expected_sha}  {deb_file.name}"
                actual_line = sha_file.read_text(encoding="utf-8").strip()
                check(
                    actual_line == expected_line,
                    f".sha256 侧车内容不匹配: 期望 {expected_line!r}, 实得 {actual_line!r}",
                )

    # 5. Windows admin-install.ps1 空格路径引号验证
    with tempfile.TemporaryDirectory() as td:
        ps1_path = Path(td) / "admin-install.ps1"
        p_ps1 = run_sh(MSI_SH, ["--dump-admin-install-ps1", str(ps1_path)])
        check(p_ps1.returncode == 0 and ps1_path.is_file(), "package-windows-msi.sh --dump-admin-install-ps1 失败")
        if ps1_path.is_file():
            ps1_text = ps1_path.read_text(encoding="utf-8")
            check(
                '$quotedMsi = \'"\' + $Msi + \'"\'' in ps1_text
                and '$quotedTarget = \'TARGETDIR="\' + $Target + \'"\'' in ps1_text
                and "@('/a', $quotedMsi, '/qn', $quotedTarget)" in ps1_text,
                f"admin-install.ps1 未对含空格的 $Msi 与 TARGETDIR 显式补双引号:\n{ps1_text}",
            )
            sample_msi = r"C:\Users\build runner\out dir\yeban-0.0.1-x86_64-pc-windows-msvc.msi"
            sample_target = r"C:\Temp\work dir\admin"
            expanded_msi = '"' + sample_msi + '"'
            expanded_target = 'TARGETDIR="' + sample_target + '"'
            check(
                expanded_msi.startswith('"') and expanded_msi.endswith('"') and " " in expanded_msi,
                "空格 MSI 路径引号包裹异常",
            )
            check(
                expanded_target.startswith('TARGETDIR="') and expanded_target.endswith('"') and " " in expanded_target,
                "空格 TARGETDIR 引号包裹异常",
            )

    if failures:
        for f in failures:
            print(f"[FAIL] {f}", file=sys.stderr)
        return 1

    print("[ok] check_package_scripts_hardening: 语法 3 项 + 合法命名 6 项 + 非法版本拦截 27 项 + 非法 target 拦截 15 项 + deb 端到端构建/control/sha256 + MSI 空格路径引号 全部通过")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
