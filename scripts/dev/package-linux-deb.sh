#!/usr/bin/env bash
# 把 workflow 已生成的 stage 目录 (yeban-app / yeban-mcp / LICENSE, 与 tar.gz 同一份)
# 打成 Debian/Ubuntu 的 .deb 安装包。
#
# 用法:
#   bash scripts/dev/package-linux-deb.sh <stage目录> <target> <输出目录>
#   bash scripts/dev/package-linux-deb.sh --print-name <target>
#
# 产物命名 (稳定模板, 下一期的 APT 仓库 / AUR yeban-bin 靠它拼 URL, 不许改形):
#   yeban-<工作区版本>-<target>.deb (+ 同名 .sha256 侧车)
#   --print-name 只打印这个文件名 (版本读根 Cargo.toml, 可用 YEBAN_PKG_VERSION 覆盖),
#   供渠道清单 (cask / PKGBUILD / reprepro 配置) 机械引用。
#
# 包布局 (按进 APT 仓库的标准写, reprepro/aptly 扫包时不该被拒):
#   /usr/lib/yeban/yeban-app, yeban-mcp, LICENSE   ← 本体 (0755 / 0755 / 0644)
#   /usr/bin/yeban, /usr/bin/yeban-mcp             ← 指向 ../lib/yeban/* 的软链
#   /usr/share/applications/dev.yeban.app.desktop  ← resources/linux/dev.yeban.app.desktop
#   /usr/share/icons/hicolor/scalable/apps/yeban.svg + {64,128,256,512}x*/apps/yeban.png
#                                                  ← assets/brand 的深色变体 (SVG/同尺寸 PNG)
#   /usr/share/mime/packages/yeban.xml             ← .yeban 工程文件的 MIME 注册
#   /usr/share/yeban/assets/templates/             ← 工厂工程模板（README + 日后 .yeban）
#   DEBIAN/control: Package/Version/Architecture/Maintainer/Section/Priority/
#                   Depends/Recommends/Installed-Size/Homepage/Description 写全。
#   .desktop 身份三一致 (Omarchy/Hyprland 的窗口↔启动器匹配靠它): 文件名
#   dev.yeban.app.desktop、StartupWMClass=dev.yeban.app、应用的 Wayland app_id
#   目标值 dev.yeban.app (与 macOS bundle id 同一个身份); Icon=yeban 用裸名。
#   ⚠ 应用侧目前**没有**显式设置 winit/Slint 的 application_id (代码里只有
#   bundle id 与数据目录在用 dev.yeban.app) —— 显式设置是应用侧待办, 不在本脚本范围。
#   Depends 的依据: README 的 Linux 构建依赖表 (dev 包 → 对应运行时包) +
#   libxkbcommon-x11-0 (真桌面首启 panic 修复时确认的运行时件, 见 ci.yml apt 清单) +
#   CI 实测 ldd (run 43820577463 Linux 腿日志): DT_NEEDED 直链只有 fontconfig/
#   freetype/asound/libstdc++/libc/libm —— xkbcommon/wayland/X11/GL 是 winit 在
#   运行时 dlopen 的, 不出现在 ldd 里但必须声明 (libstdc++6 即据此补入;
#   expat/z/bz2/png/brotli 是 fontconfig/freetype 的传递依赖, 不重复声明)。
#   音频依赖按实际核实: 引擎是 cpal + ALSA (根 Cargo.toml 的 cpal 无 jack
#   feature; midir 的 jack 按 ADR-0006 D6 全仓禁用; 无 pipewire 链接) ⇒
#   硬依赖只有 libasound; PipeWire 桌面上的 ALSA 路由件 (pipewire-alsa +
#   wireplumber) 放 Recommends (apt 默认会装), 不把 pipewire-jack/pipewire-pulse
#   写成依赖 —— 应用没有 JACK/Pulse 代码路径, 写了就是假依赖。
#   本脚本会顺带打印 stage 里 yeban-app 的 ldd (有 ldd 时), 让 CI 日志留一份
#   真实链接证据, 供日后收紧 Depends 时对账 —— 打印只是证据, 不作判据。
#
# 自验 (脚本内, 失败即非零退出):
#   ① dpkg-deb --info 的 Package/Version/Architecture 与预期逐字对账;
#   ② dpkg-deb --contents 含全部预期路径;
#   ③ dpkg-deb -x 解开后断言本体可执行、软链指向正确、desktop/图标在位。
#   YEBAN_PKG_INSTALL_TEST=1 时另做实装验证 (CI 用): sudo dpkg -i →
#   /usr/lib/yeban/yeban-app --version 与清单版本对账 → sudo dpkg -r yeban 卸载。
#   本机/盒上单跑不要开这个开关 (会真装真卸系统包)。
#
# 退出码: 0 = 成功; 非 0 = 任一环节失败 (响亮失败, 不产出半成品当成功)。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

step() { printf '\n[pkg-deb] %s\n' "$*"; }
die() { printf '[pkg-deb][fail] %s\n' "$*" >&2; exit 1; }

validate_version() {
  local v="$1"
  [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?(\+[0-9A-Za-z.]+)?$ ]] \
    || die "非法版本号 '$v': 必须符合 SemVer x.y.z[-prerelease][+build] 形态（严禁换行、冒号或引号）"
}

validate_target() {
  case "$1" in
    x86_64-unknown-linux-gnu) deb_arch="amd64" ;;
    aarch64-unknown-linux-gnu) deb_arch="arm64" ;;
    *) die "目标 '$1' 没有 DEB 架构映射 (只支持 x86_64-unknown-linux-gnu / aarch64-unknown-linux-gnu)" ;;
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
  target="${2:?用法: package-linux-deb.sh --print-name <target>}"
  validate_target "$target"
  ver="$(read_version)"
  printf 'yeban-%s-%s.deb\n' "$ver" "$target"
  exit 0
fi

stage_dir="${1:?用法: package-linux-deb.sh <stage目录> <target> <输出目录>}"
target="${2:?用法: package-linux-deb.sh <stage目录> <target> <输出目录>}"
out_dir="${3:?用法: package-linux-deb.sh <stage目录> <target> <输出目录>}"

validate_target "$target"

command -v dpkg-deb >/dev/null 2>&1 || die "缺少工具: dpkg-deb"
for f in yeban-app yeban-mcp LICENSE; do
  [[ -f "$stage_dir/$f" ]] || die "stage 目录缺 $f: $stage_dir"
done
[[ -x "$stage_dir/yeban-app" ]] || die "stage 里的 yeban-app 不可执行: $stage_dir/yeban-app"
[[ -x "$stage_dir/yeban-mcp" ]] || die "stage 里的 yeban-mcp 不可执行: $stage_dir/yeban-mcp"

version="$(read_version)"
[[ -n "$version" ]] || die "版本解析为空"
name="yeban-${version}-${target}"

# --- ldd 证据 (只打印, 不作判据) --------------------------------------------
if command -v ldd >/dev/null 2>&1; then
  step "ldd $stage_dir/yeban-app (链接证据, 供 Depends 对账)"
  ldd "$stage_dir/yeban-app" || printf '[pkg-deb] ldd 返回非零 (静态链接或缺库), 仅记录\n'
fi

# --- 装配 --------------------------------------------------------------------
work_base="${TMPDIR:-/tmp}"
work="$(mktemp -d "$work_base/yeban-deb.XXXXXX")"
# 若 /tmp 挂载了 noexec，dpkg-deb -x 解开后的 [[ -x ]] 自验会恒为假 ⇒ 退回输出目录旁建工作区
probe_exec="$work/.exec_probe"
printf '#!/bin/sh\n' > "$probe_exec" && chmod 0755 "$probe_exec"
if [[ ! -x "$probe_exec" ]]; then
  rm -rf "$work"
  mkdir -p "$out_dir"
  work="$(mktemp -d "$out_dir/.yeban-deb-work.XXXXXX")"
fi
rm -f "$work/.exec_probe"
trap 'rm -rf "$work"' EXIT
root="$work/root"
mkdir -p "$root/usr/lib/yeban" "$root/usr/bin" \
         "$root/usr/share/applications" \
         "$root/usr/share/icons/hicolor/scalable/apps" \
         "$root/usr/share/mime/packages" \
         "$root/DEBIAN"
for size in 64 128 256 512; do
  mkdir -p "$root/usr/share/icons/hicolor/${size}x${size}/apps"
done

install -m 0755 "$stage_dir/yeban-app" "$root/usr/lib/yeban/yeban-app"
install -m 0755 "$stage_dir/yeban-mcp" "$root/usr/lib/yeban/yeban-mcp"
install -m 0644 "$stage_dir/LICENSE"   "$root/usr/lib/yeban/LICENSE"
ln -s ../lib/yeban/yeban-app "$root/usr/bin/yeban"
ln -s ../lib/yeban/yeban-mcp "$root/usr/bin/yeban-mcp"
install -m 0644 "$repo_root/resources/linux/dev.yeban.app.desktop" \
  "$root/usr/share/applications/dev.yeban.app.desktop"
install -m 0644 "$repo_root/resources/linux/yeban-mime.xml" \
  "$root/usr/share/mime/packages/yeban.xml"
# 工厂工程模板：运行时 factory_template_root 读 /usr/share/yeban/assets/templates。
install -d "$root/usr/share/yeban/assets/templates"
cp -a "$repo_root/assets/templates/." "$root/usr/share/yeban/assets/templates/"
[[ -f "$root/usr/share/yeban/assets/templates/README.md" ]] \
  || die "deb 暂存树缺工厂模板 README: $root/usr/share/yeban/assets/templates/README.md"
# 图标: 裸名 yeban; SVG 进 scalable, PNG 按品牌资产现有尺寸进各档
# (品牌集只有 32/64/128/256/512 的 PNG, 没有 48 —— scalable SVG 覆盖该档)。
install -m 0644 "$repo_root/assets/brand/yeban-dark-512.svg" \
  "$root/usr/share/icons/hicolor/scalable/apps/yeban.svg"
for size in 64 128 256 512; do
  install -m 0644 "$repo_root/assets/brand/png/yeban-dark-$size.png" \
    "$root/usr/share/icons/hicolor/${size}x${size}/apps/yeban.png"
done

# mktemp -d 是 0700, dpkg-deb 要求 control 目录权限在 0755..0775 之间 ⇒ 全树目录归一。
find "$root" -type d -exec chmod 0755 {} +

installed_size="$(du -sk --apparent-size "$root/usr" | cut -f1)"

cat > "$root/DEBIAN/control" <<EOF
Package: yeban
Version: $version
Architecture: $deb_arch
Maintainer: Yeban Contributors <yeban@wangda.today>
Section: sound
Priority: optional
Depends: libc6, libstdc++6, libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libx11-6, libgl1, libasound2t64 | libasound2
Recommends: pipewire-alsa, wireplumber, fonts-noto-cjk
Installed-Size: $installed_size
Homepage: https://yeban.wangda.today
Description: Yeban (夜半) - AI-native digital audio workstation
 Yeban is an AI-native DAW written in Rust: a Slint desktop application
 (yeban-app) plus a headless MCP server (yeban-mcp). Projects are an
 append-only op log folded into the document, so every edit is auditable
 and replayable.
 .
 This package installs the release build (default feature set) into
 /usr/lib/yeban with /usr/bin/yeban and /usr/bin/yeban-mcp launchers.
EOF

mkdir -p "$out_dir"
step "dpkg-deb --build → $out_dir/$name.deb"
dpkg-deb --root-owner-group --build "$root" "$out_dir/$name.deb"
write_sha256_sidecar "$out_dir" "$name.deb"

# --- 自验 ①②③ ---------------------------------------------------------------
step "自验: dpkg-deb --info / --contents / -x"
info="$(dpkg-deb --info "$out_dir/$name.deb")"
printf '%s\n' "$info"
for want in "Package: yeban" "Version: $version" "Architecture: $deb_arch"; do
  printf '%s\n' "$info" | grep -q "^ $want\$" \
    || die "control 字段对不上: 期望 '$want'"
done
contents="$(dpkg-deb --contents "$out_dir/$name.deb")"
printf '%s\n' "$contents"
for want in "./usr/lib/yeban/yeban-app" "./usr/lib/yeban/yeban-mcp" \
            "./usr/lib/yeban/LICENSE" "./usr/bin/yeban" "./usr/bin/yeban-mcp" \
            "./usr/share/applications/dev.yeban.app.desktop" \
            "./usr/share/mime/packages/yeban.xml" \
            "./usr/share/yeban/assets/templates/README.md" \
            "./usr/share/yeban/assets/templates/manifest.json" \
            "./usr/share/icons/hicolor/scalable/apps/yeban.svg" \
            "./usr/share/icons/hicolor/64x64/apps/yeban.png" \
            "./usr/share/icons/hicolor/128x128/apps/yeban.png" \
            "./usr/share/icons/hicolor/256x256/apps/yeban.png" \
            "./usr/share/icons/hicolor/512x512/apps/yeban.png"; do
  printf '%s\n' "$contents" | grep -q -- "$want" \
    || die "--contents 缺预期路径: $want"
done
extract="$work/extract"
dpkg-deb -x "$out_dir/$name.deb" "$extract"
[[ -x "$extract/usr/lib/yeban/yeban-app" ]] || die "解开后 yeban-app 不可执行"
[[ -x "$extract/usr/lib/yeban/yeban-mcp" ]] || die "解开后 yeban-mcp 不可执行"
[[ -L "$extract/usr/bin/yeban" ]] || die "解开后 /usr/bin/yeban 不是软链"
[[ "$(readlink "$extract/usr/bin/yeban")" == "../lib/yeban/yeban-app" ]] \
  || die "/usr/bin/yeban 软链指向不对: $(readlink "$extract/usr/bin/yeban")"
[[ "$(readlink "$extract/usr/bin/yeban-mcp")" == "../lib/yeban/yeban-mcp" ]] \
  || die "/usr/bin/yeban-mcp 软链指向不对"
[[ -d "$extract/usr/share/yeban/assets/templates" ]] \
  || die "解开后缺工厂模板目录: usr/share/yeban/assets/templates"
echo "[pkg-deb] 结构自验通过"

# --- 实装验证 (仅 CI: YEBAN_PKG_INSTALL_TEST=1) ------------------------------
if [[ "${YEBAN_PKG_INSTALL_TEST:-}" == "1" ]]; then
  step "实装验证: sudo dpkg -i → --version 对账 → sudo dpkg -r yeban"
  sudo dpkg -i "$out_dir/$name.deb"
  got="$(/usr/lib/yeban/yeban-app --version)"
  echo "installed --version: $got"
  [[ "$got" == "yeban-app $version" ]] \
    || die "实装后 --version '$got' != 'yeban-app $version'"
  sudo dpkg -r yeban
  echo "[pkg-deb] 实装验证通过 (已卸载)"
fi

bytes="$(stat -c %s "$out_dir/$name.deb" 2>/dev/null || stat -f %z "$out_dir/$name.deb")"
sha="$(cut -d' ' -f1 < "$out_dir/$name.deb.sha256")"
printf '\n[pkg-deb] 产物: %s | %s B | sha256 %s\n' "$name.deb" "$bytes" "$sha"
