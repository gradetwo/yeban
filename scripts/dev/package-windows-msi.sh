#!/usr/bin/env bash
# 把 workflow 已生成的 stage 目录 (yeban-app.exe / yeban-mcp.exe / LICENSE,
# 与 tar.gz 同一份) 打成 Windows .msi 安装包 (WiX Toolset v4+)。
#
# 用法 (在 windows runner 的 Git Bash 里跑):
#   bash scripts/dev/package-windows-msi.sh <stage目录> <target> <输出目录>
#   bash scripts/dev/package-windows-msi.sh --print-name <target>
#
# 产物命名 (稳定模板, 渠道清单靠它拼 URL, 不许改形):
#   yeban-<工作区版本>-x86_64-pc-windows-msvc.msi (+ 同名 .sha256 侧车)
#
# 安装行为 (源文件 resources/windows/yeban.wxs, 产品身份 GUID 写死在那里):
#   装两 exe + LICENSE 到 Program Files\Yeban、开始菜单快捷方式、注册卸载项、
#   MajorUpgrade 就地升级。perMachine, 需要管理员权限安装 —— 桌面 DAW 的常态。
#
# 工具来源 (仓库纪律: 只用发行版工具, 不下随机二进制):
#   PATH 里有现成的就用; 没有就 `choco install wixtoolset -y` (Chocolatey 是
#   runner 自带的包管理器), 装完再找一遍; 仍找不到 ⇒ 响亮失败, 不静默换别的打包路。
#   ⚠ 实测 (2026-11-16 nightly 分支 run 43820577463 Windows 腿日志): choco 的
#   wixtoolset 是 **3.14.1** (candle/light, 没有 v4 的 wix CLI)。故两条路都支持:
#   有 v4+ 的 `wix` 就编 yeban.wxs (v4 语法); 只有 v3 就 candle+light 编
#   yeban-v3.wxs (同一产品身份的 v3 方言)。用哪条路由日志如实打印。
#   WiX v5+ 首次 build 会要求接受 OSMF EULA: v4 路第一次 build 若失败, 带
#   `-acceptEula wix0001` 重试一次 (重试的输出原样留在日志里, 不吞)。
#
# 版本: 根 Cargo.toml 的 [workspace.package] version; MSI 的 ProductVersion
#   只接受数字形态 ⇒ 剥掉 `-预发布后缀` 再写进 .wxs (工作区现版本 0.0.1 无后缀;
#   真出现后缀时, MSI 版本与清单版本在此分叉, 发布说明须注明)。
#
# 自验 (脚本内, 失败即非零退出):
#   msiexec /a 管理安装 (只解包不注册) 到临时目录 —— 经 PowerShell
#   Start-Process -Wait 调用 (msiexec 本体不等完成, 裸调会假绿) ——
#   断言 yeban-app.exe / yeban-mcp.exe 落地, 再跑解出的 yeban-app.exe
#   --version 与清单版本逐字对账。
#
# 退出码: 0 = 成功; 非 0 = 任一环节失败。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

step() { printf '\n[pkg-msi] %s\n' "$*"; }
die() { printf '[pkg-msi][fail] %s\n' "$*" >&2; exit 1; }

read_version() {
  if [[ -n "${YEBAN_PKG_VERSION:-}" ]]; then
    printf '%s' "$YEBAN_PKG_VERSION"
    return
  fi
  python3 - "$repo_root/Cargo.toml" <<'PY'
import pathlib, sys, tomllib
print(tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))["workspace"]["package"]["version"])
PY
}

# --- --print-name: 渠道清单机械引用的命名模板 -------------------------------
if [[ "${1:-}" == "--print-name" ]]; then
  target="${2:?用法: package-windows-msi.sh --print-name <target>}"
  printf 'yeban-%s-%s.msi\n' "$(read_version)" "$target"
  exit 0
fi

stage_dir="${1:?用法: package-windows-msi.sh <stage目录> <target> <输出目录>}"
target="${2:?用法: package-windows-msi.sh <stage目录> <target> <输出目录>}"
out_dir="${3:?用法: package-windows-msi.sh <stage目录> <target> <输出目录>}"

[[ "$target" == "x86_64-pc-windows-msvc" ]] \
  || die "MSI 只支持 x86_64-pc-windows-msvc, 收到: $target"
for f in yeban-app.exe yeban-mcp.exe LICENSE; do
  [[ -f "$stage_dir/$f" ]] || die "stage 目录缺 $f: $stage_dir"
done

version="$(read_version)"
[[ -n "$version" ]] || die "版本解析为空"
msi_version="${version%%-*}"
[[ "$msi_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || die "版本 '$version' 剥后缀后 '$msi_version' 不是 MSI 可接受的 x.y.z 形态"
name="yeban-${version}-${target}"

stage_abs="$(cd "$stage_dir" && pwd)"
mkdir -p "$out_dir"
out_abs="$(cd "$out_dir" && pwd)"

# --- 1. 找到 WiX (v4 的 wix CLI 或 v3 的 candle, 见头部实测注) ----------------
find_wix4() {
  if command -v wix >/dev/null 2>&1; then command -v wix; return 0; fi
  local d
  for d in "/c/Program Files (x86)"/WiX\ Toolset\ v*/bin "/c/Program Files"/WiX\ Toolset\ v*/bin; do
    [[ -x "$d/wix.exe" ]] && { printf '%s' "$d/wix.exe"; return 0; }
  done
  return 1
}
find_candle() {
  if command -v candle.exe >/dev/null 2>&1; then command -v candle.exe; return 0; fi
  local d
  # WiX v3 安装器会设 WIX 环境变量指向安装根。
  if [[ -n "${WIX:-}" ]]; then
    d="$(cygpath -u "$WIX" 2>/dev/null || true)"
    [[ -n "$d" && -x "$d/bin/candle.exe" ]] && { printf '%s' "$d/bin/candle.exe"; return 0; }
  fi
  for d in "/c/Program Files (x86)"/WiX\ Toolset\ v3*/bin "/c/Program Files"/WiX\ Toolset\ v3*/bin; do
    [[ -x "$d/candle.exe" ]] && { printf '%s' "$d/candle.exe"; return 0; }
  done
  return 1
}

WIX4="$(find_wix4 || true)"
CANDLE="$(find_candle || true)"
if [[ -z "$WIX4" && -z "$CANDLE" ]]; then
  step "没有现成的 WiX ⇒ choco install wixtoolset"
  command -v choco >/dev/null 2>&1 || die "没有 WiX 也没有 choco, 无法按既定路安装 WiX"
  choco install wixtoolset -y --no-progress
  WIX4="$(find_wix4 || true)"
  CANDLE="$(find_candle || true)"
fi
[[ -n "$WIX4" || -n "$CANDLE" ]] \
  || die "choco install wixtoolset 后仍找不到 wix.exe (v4) 或 candle.exe (v3)"

# --- 2. 生成 .wxs 并 build ---------------------------------------------------
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

src_win="$(cygpath -w "$stage_abs")"
out_win="$(cygpath -w "$out_abs/$name.msi")"

if [[ -n "$WIX4" ]]; then
  echo "[pkg-msi] 工具链: WiX v4+ CLI: $WIX4"
  "$WIX4" --version || true
  sed "s/__YEBAN_VERSION__/$msi_version/g" \
    "$repo_root/resources/windows/yeban.wxs" > "$work/yeban.wxs"
  grep -q -- "$msi_version" "$work/yeban.wxs" || die ".wxs 版本占位符替换失败"
  step "wix build → $out_dir/$name.msi"
  if ! "$WIX4" build -arch x64 -d "SourceDir=$src_win" -o "$out_win" "$(cygpath -w "$work/yeban.wxs")"; then
    echo "[pkg-msi] 首次 build 失败, 带 -acceptEula wix0001 重试一次 (WiX v5+ 的 EULA 门)"
    "$WIX4" build -acceptEula wix0001 -arch x64 -d "SourceDir=$src_win" -o "$out_win" "$(cygpath -w "$work/yeban.wxs")" \
      || die "wix build 两次都失败 (原因见上面输出)"
  fi
else
  LIGHT="$(dirname "$CANDLE")/light.exe"
  [[ -x "$LIGHT" ]] || die "有 candle.exe 但同目录没有 light.exe: $CANDLE"
  echo "[pkg-msi] 工具链: WiX v3: $CANDLE + $LIGHT"
  "$CANDLE" -help >/dev/null 2>&1 || true
  sed "s/__YEBAN_VERSION__/$msi_version/g" \
    "$repo_root/resources/windows/yeban-v3.wxs" > "$work/yeban.wxs"
  grep -q -- "$msi_version" "$work/yeban.wxs" || die ".wxs 版本占位符替换失败"
  step "candle + light → $out_dir/$name.msi"
  ( cd "$work" && "$CANDLE" -arch x64 -d"SourceDir=$src_win" yeban.wxs \
      && "$LIGHT" -o "$out_win" yeban.wixobj ) \
    || die "candle/light 失败 (原因见上面输出)"
fi
[[ -f "$out_abs/$name.msi" ]] || die "build 退出 0 但 MSI 不在: $out_abs/$name.msi"
# WiX (v3 light / v4 build 都会) 在 MSI 旁边留一个 $name.wixpdb 调试符号文件;
# 它不是发布物, 不删会被 workflow 的 dist/* 一并上传到 Release 里
# (2026-11-16 run 43823130284 的 Windows artifact 实证混入)。
rm -f "$out_abs/$name.wixpdb"
( cd "$out_abs" && python3 -c "import hashlib; print(hashlib.sha256(open('$name.msi','rb').read()).hexdigest() + '  $name.msi')" > "$name.msi.sha256" )

# --- 3. 自验: msiexec /a 管理安装 + --version 对账 ---------------------------
step "自验: msiexec /a 解包到临时目录"
cat > "$work/admin-install.ps1" <<'PS1'
param([string]$Msi, [string]$Target)
$p = Start-Process msiexec -ArgumentList @('/a', $Msi, '/qn', "TARGETDIR=$Target") -Wait -PassThru
exit $p.ExitCode
PS1
admin_dir="$work/admin"
powershell.exe -NoProfile -ExecutionPolicy Bypass \
  -File "$(cygpath -w "$work/admin-install.ps1")" \
         "$out_win" "$(cygpath -w "$admin_dir")" \
  || die "msiexec /a 管理安装失败 (退出码见上)"
[[ -f "$admin_dir/Yeban/yeban-app.exe" ]] || die "管理安装后 yeban-app.exe 没落地: $admin_dir/Yeban"
[[ -f "$admin_dir/Yeban/yeban-mcp.exe" ]] || die "管理安装后 yeban-mcp.exe 没落地: $admin_dir/Yeban"
got="$("$admin_dir/Yeban/yeban-app.exe" --version)"
echo "msi --version: $got"
[[ "$got" == "yeban-app $version" ]] \
  || die "MSI 里二进制 --version '$got' != 'yeban-app $version'"
echo "[pkg-msi] 自验通过"

bytes="$(stat -c %s "$out_abs/$name.msi")"
sha="$(cut -d' ' -f1 < "$out_abs/$name.msi.sha256")"
printf '\n[pkg-msi] 产物: %s | %s B | sha256 %s\n' "$name.msi" "$bytes" "$sha"
