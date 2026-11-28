#!/usr/bin/env bash
# 把 workflow 已生成的 stage 目录里的 yeban-app (与 tar.gz 同一份) 装配成
# Yeban.app (复用 scripts/dev/bundle-macos.sh), 再打成 macOS .dmg 安装包。
#
# 用法:
#   bash scripts/dev/package-macos-dmg.sh <stage目录> <target> <输出目录>
#   bash scripts/dev/package-macos-dmg.sh --print-name <target>
#
# 产物命名 (稳定模板, 下一期的 Homebrew cask 靠它拼 URL, 不许改形):
#   yeban-<工作区版本>-<target>.dmg (+ 同名 .sha256 侧车)
#
# DMG 内容: Yeban.app + Applications 软链 (拖拽安装的标准布局)。
#
# 签名现状 (如实): bundle-macos.sh 用 ad-hoc 签名 (codesign --sign -),
# **没有** Developer ID、**没有** 公证 ⇒ Gatekeeper 会拦首次打开, 用户须手动
# 放行。这与 nightly 既有的"未签名"口径一致; 要签名分发须先有证书, 不在本脚本范围。
#
# 自验 (脚本内, 失败即非零退出):
#   ① hdiutil verify 校验镜像本身;
#   ② 只读挂载后断言 Yeban.app 在内, 且 codesign --verify --deep --strict 通过;
#   ③ 原生腿另跑包内二进制 --version 与清单版本对账; 交叉腿
#      (YEBAN_PKG_SKIP_RUN=1, arm64 runner 上装配 x86_64 包) 跳过 ③, 只验
#      结构与签名 —— x86_64 二进制不保证能在 arm64 上本地执行。
#
# 退出码: 0 = 成功; 非 0 = 任一环节失败。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

step() { printf '\n[pkg-dmg] %s\n' "$*"; }
die() { printf '[pkg-dmg][fail] %s\n' "$*" >&2; exit 1; }

validate_version() {
  local v="$1"
  [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?(\+[0-9A-Za-z.]+)?$ ]] \
    || die "非法版本号 '$v': 必须符合 SemVer x.y.z[-prerelease][+build] 形态"
}

validate_target() {
  case "$1" in
    aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) die "目标 '$1' 不受 macOS DMG 支持 (只支持 aarch64-apple-darwin / x86_64-apple-darwin)" ;;
  esac
}

read_version() {
  local v
  if [[ -n "${YEBAN_PKG_VERSION:-}" ]]; then
    v="$YEBAN_PKG_VERSION"
  else
    v="$(python3 - "$repo_root/Cargo.toml" <<'PY'
import pathlib, sys, tomllib
print(tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))["workspace"]["package"]["version"])
PY
)"
  fi
  [[ -n "$v" ]] || die "版本解析为空"
  validate_version "$v"
  printf '%s' "$v"
}

write_sha256_sidecar() {
  local dir="$1" fname="$2"
  python3 - "$dir" "$fname" <<'PY'
import hashlib, pathlib, sys
d = pathlib.Path(sys.argv[1])
fname = sys.argv[2]
h = hashlib.sha256()
with open(d / fname, "rb") as f:
    for chunk in iter(lambda: f.read(65536), b""):
        h.update(chunk)
(d / (fname + ".sha256")).write_text(f"{h.hexdigest()}  {fname}\n", encoding="utf-8")
PY
}

# --- --print-name: 渠道清单机械引用的命名模板 -------------------------------
if [[ "${1:-}" == "--print-name" ]]; then
  target="${2:?用法: package-macos-dmg.sh --print-name <target>}"
  validate_target "$target"
  ver="$(read_version)"
  printf 'yeban-%s-%s.dmg\n' "$ver" "$target"
  exit 0
fi

stage_dir="${1:?用法: package-macos-dmg.sh <stage目录> <target> <输出目录>}"
target="${2:?用法: package-macos-dmg.sh <stage目录> <target> <输出目录>}"
out_dir="${3:?用法: package-macos-dmg.sh <stage目录> <target> <输出目录>}"

validate_target "$target"
version="$(read_version)"

[[ "$(uname -s)" == "Darwin" ]] || die "本脚本只适用于 macOS (uname -s = $(uname -s))"

[[ -x "$stage_dir/yeban-app" ]] || die "stage 里的 yeban-app 不存在或不可执行: $stage_dir/yeban-app"
for tool in hdiutil codesign python3; do
  command -v "$tool" >/dev/null 2>&1 || die "缺少工具: $tool"
done

version="$(read_version)"
[[ -n "$version" ]] || die "版本解析为空"
name="yeban-${version}-${target}"

work="$(mktemp -d)"
mount="$work/mount"
mounted=0
cleanup() {
  if [[ "$mounted" == "1" ]]; then
    hdiutil detach "$mount" >/dev/null 2>&1 || hdiutil detach "$mount" -force >/dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# --- 1. 装配 Yeban.app (复用 bundle-macos.sh 的既有路) -----------------------
step "装配 Yeban.app (bundle-macos.sh, 二进制 = stage/yeban-app)"
YEBAN_BUNDLE_BINARY="$stage_dir/yeban-app" \
YEBAN_BUNDLE_OUT="$work/Yeban.app" \
YEBAN_BUNDLE_SKIP_SMOKE="${YEBAN_PKG_SKIP_RUN:-}" \
  bash "$repo_root/scripts/dev/bundle-macos.sh"
[[ -d "$work/Yeban.app" ]] || die "bundle-macos.sh 跑完但 Yeban.app 不在: $work/Yeban.app"

# --- 2. 打 DMG ---------------------------------------------------------------
dmgroot="$work/dmgroot"
mkdir -p "$dmgroot"
cp -R "$work/Yeban.app" "$dmgroot/Yeban.app"
ln -s /Applications "$dmgroot/Applications"

mkdir -p "$out_dir"
step "hdiutil create → $out_dir/$name.dmg"
hdiutil create -volname "Yeban $version" -srcfolder "$dmgroot" -ov -format UDZO \
  "$out_dir/$name.dmg"
write_sha256_sidecar "$out_dir" "$name.dmg"

# --- 3. 自验 -----------------------------------------------------------------
step "自验: hdiutil verify"
hdiutil verify "$out_dir/$name.dmg"

step "自验: 挂载后断言 Yeban.app 在内 + codesign --verify"
mkdir -p "$mount"
hdiutil attach "$out_dir/$name.dmg" -readonly -nobrowse -mountpoint "$mount"
mounted=1
[[ -d "$mount/Yeban.app" ]] || die "DMG 里没有 Yeban.app"
[[ -L "$mount/Applications" ]] || die "DMG 里没有 Applications 软链"
codesign --verify --deep --strict --verbose=2 "$mount/Yeban.app"
if [[ "${YEBAN_PKG_SKIP_RUN:-}" == "1" ]]; then
  echo "[pkg-dmg] 交叉腿 (YEBAN_PKG_SKIP_RUN=1): 跳过包内二进制 --version, 只验结构与签名"
else
  got="$("$mount/Yeban.app/Contents/MacOS/yeban" --version)"
  echo "bundle --version: $got"
  [[ "$got" == "yeban-app $version" ]] \
    || die "包内二进制 --version '$got' != 'yeban-app $version'"
fi
hdiutil detach "$mount"
mounted=0
echo "[pkg-dmg] 自验通过"

bytes="$(stat -f %z "$out_dir/$name.dmg")"
sha="$(cut -d' ' -f1 < "$out_dir/$name.dmg.sha256")"
printf '\n[pkg-dmg] 产物: %s | %s B | sha256 %s\n' "$name.dmg" "$bytes" "$sha"
